//! Tier 3 of the Lean-4 conformance corpus — INDUCTIVES AND STRUCTURES, re-read as a
//! capability matrix rather than a translator spec. Twenty-four cases; the verdicts live in
//! `crates/logicaffeine_tests/tests/fixtures/lean_conformance/verdicts/t03_inductives.json`.
//!
//! This is the tier where LOGOS is strongest and where the corpus report is most out of date. Mutual
//! blocks register transactionally with cross-block positivity; nested inductives compile to a
//! mutual block plus KERNEL-CHECKED isos; structures carry definitional eta; indexed families
//! keep their per-constructor indices. The genuine holes are narrow and specific:
//!
//!   * `add_structure` hard-codes `Sort(Type 0)` and registers its constructor through the
//!     UNCHECKED `add_constructor` — so `Prop`- and `Type u≥1`-valued records are unreachable.
//!   * `add_mutual_inductives` never compares the members' parameter counts; the check lives in
//!     `derive_mutual_recursor` and fires long after the block is in the context.
//!   * the indexed `Match` has no index unifier: every constructor owes a branch even when its
//!     indices provably cannot match the scrutinee's.
//!   * nothing generates `noConfusion`/`injEq`, nothing derives a `DecidableEq`, and `Quot` has
//!     no dependent eliminator.
//!
//! Two cases are DIVERGE and say so in their own doc comments: projections are ordinary global
//! definitions rather than a kernel `Expr.proj` (T03-011), and `Match` is primitive with the
//! recursor derived from it — the inverse of Lean (T03-021).
//!
//! T03-005 (params/indices misdeclaration) is a confirmed LOGOS defect whose reproduction lives
//! in `tests/lean_conformance_defects.rs`; its verdict points there rather than carrying a second copy.
//! T03-017's `@guard` about the subsingleton criterion's positional arity skip is defect 2, whose
//! reproduction is likewise there and is claimed by T01-014 — so T03-017's own test decides the
//! rest of its oracle (the canonical gate table) instead of duplicating it.

use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{
    check_positivity_for_test, defeq_for_test, derive_recursor, double_check, infer_type,
    instantiate_universes, is_subtype, normalize, resolve_instance, Context, DoubleCheck,
    KernelError, MetaCtx, MutualInductive, NestedDecl, ParamKind, Term, Universe,
};

// =============================================================================
// PER-FILE HELPERS (house style: every kernel test declares its own)
// =============================================================================

fn g(n: &str) -> Term {
    Term::Global(n.to_string())
}
fn v(n: &str) -> Term {
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
fn match_(d: Term, motive: Term, cases: Vec<Term>) -> Term {
    Term::Match {
        discriminant: Box::new(d),
        motive: Box::new(motive),
        cases,
    }
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
fn succ(n: Term) -> Term {
    app(g("Succ"), n)
}
fn std_ctx() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx
}

/// Alpha-aware type equality (binder names may differ, `Π(n:Nat).Bool` ≡ `Nat→Bool`).
fn same_type(ctx: &Context, a: &Term, b: &Term) -> bool {
    is_subtype(ctx, a, b) && is_subtype(ctx, b, a)
}

fn ctor_names(ctx: &Context, ind: &str) -> Vec<String> {
    ctx.get_constructors(ind)
        .iter()
        .map(|(n, _)| n.to_string())
        .collect()
}

fn count_leading_pis(t: &Term) -> usize {
    match t {
        Term::Pi { body_type, .. } => 1 + count_leading_pis(body_type),
        _ => 0,
    }
}

/// The domain of the `n`-th leading `Π` of `t` (0-based), or `None` if there are fewer.
fn nth_pi_domain(t: &Term, n: usize) -> Option<Term> {
    let mut cur = t;
    let mut i = 0;
    while let Term::Pi {
        param_type,
        body_type,
        ..
    } = cur
    {
        if i == n {
            return Some((**param_type).clone());
        }
        cur = body_type;
        i += 1;
    }
    None
}

/// Strip the leading `λ`s of a term and return what is underneath.
fn peel_lambdas(t: &Term) -> &Term {
    let mut cur = t;
    while let Term::Lambda { body, .. } = cur {
        cur = body;
    }
    cur
}

// =============================================================================
// SHARED REGISTRATIONS
// =============================================================================

/// `Colour := red | green | blue` — three nullary constructors at `Type 0`, registered through
/// the CHECKED path in Lean's declaration order.
fn register_colour(ctx: &mut Context) {
    ctx.add_inductive("Colour", ty0());
    for c in ["red", "green", "blue"] {
        ctx.add_constructor_checked(c, "Colour", g("Colour"))
            .unwrap_or_else(|e| panic!("a nullary enum constructor must register: {e:?}"));
    }
}

/// `Vect (A : Type 0) : Nat → Type 0` — one parameter, one index, a recursive constructor whose
/// index grows. `vnil : Vect A Zero`, `vcons : Π(A).Π(n). A → Vect A n → Vect A (Succ n)`.
fn register_vect(ctx: &mut Context) {
    ctx.add_indexed_inductive("Vect", pi("A", ty0(), arrow(nat(), ty0())), 1);
    let vect = |a: Term, n: Term| apps(g("Vect"), &[a, n]);
    ctx.add_constructor("vnil", "Vect", pi("A", ty0(), vect(v("A"), g("Zero"))));
    ctx.add_constructor(
        "vcons",
        "Vect",
        pi(
            "A",
            ty0(),
            pi(
                "n",
                nat(),
                arrow(
                    v("A"),
                    arrow(vect(v("A"), v("n")), vect(v("A"), succ(v("n")))),
                ),
            ),
        ),
    );
}

/// `Tree := leaf : Nat → Tree | node : Tree → Tree → Tree` — a recursive `Type 0` inductive with
/// a non-uniform constructor telescope (one stored value, two recursive occurrences).
fn register_tree(ctx: &mut Context) {
    ctx.add_inductive("Tree", ty0());
    ctx.add_constructor_checked("leaf", "Tree", arrow(nat(), g("Tree")))
        .expect("leaf is strictly positive");
    ctx.add_constructor_checked(
        "node",
        "Tree",
        arrow(g("Tree"), arrow(g("Tree"), g("Tree"))),
    )
    .expect("node is strictly positive");
}

// =============================================================================
// T03-001 · Simple enum — registration order IS the match order
// =============================================================================

#[test]
fn an_enum_matches_positionally_in_its_registration_order() {
    let mut ctx = std_ctx();
    register_colour(&mut ctx);

    // The declaration's whole content is the ORDER. `Match` has no patterns and no tags:
    // `cases[i]` is the branch for `get_constructors(ind)[i]`.
    assert_eq!(
        ctor_names(&ctx, "Colour"),
        vec!["red", "green", "blue"],
        "get_constructors must return Lean's declaration order, not a HashMap's"
    );

    // An enum lives in `Type 0`, not `Prop` — a `Prop`-sorted enum would trip the
    // large-elimination gate at every use site instead of at the declaration.
    assert_eq!(
        ctx.get_global("Colour"),
        Some(&ty0()),
        "a field-less enum still lives in Type 0, never Prop"
    );

    // `code : Colour → Nat` with three pairwise-distinguishable branches. Permuting the
    // registration would typecheck perfectly and answer wrongly — so the assertion is on the
    // COMPUTED value, per constructor.
    let code = lam(
        "c",
        g("Colour"),
        match_(
            v("c"),
            lam("_", g("Colour"), nat()),
            vec![g("Zero"), succ(g("Zero")), succ(succ(g("Zero")))],
        ),
    );
    let code_ty = infer_type(&ctx, &code).expect("a positional match over an enum must typecheck");
    assert!(
        same_type(&ctx, &code_ty, &arrow(g("Colour"), nat())),
        "code : Colour → Nat, got {code_ty}"
    );
    match double_check(&ctx, &code) {
        DoubleCheck::Agreed => {}
        other => panic!("both kernels must certify a positional enum match, got {other:?}"),
    }

    for (i, c) in ["red", "green", "blue"].iter().enumerate() {
        let want = (0..i).fold(g("Zero"), |acc, _| succ(acc));
        assert_eq!(
            normalize(&ctx, &app(code.clone(), g(c))),
            want,
            "branch {i} must be selected by constructor '{c}' — a permuted registration is \
             type-invisible and only the computed value catches it"
        );
    }
}

// =============================================================================
// T03-002 · Recursive inductive, and the prelude's hard-coded literal bridge
// =============================================================================

#[test]
fn a_peano_inductive_recurses_while_only_the_prelude_nat_bridges_literals() {
    let mut ctx = std_ctx();
    // `MyNat := mzero | msucc : MyNat → MyNat` — Peano-shaped, and NOT the prelude's `Nat`.
    ctx.add_inductive("MyNat", ty0());
    ctx.add_constructor_checked("mzero", "MyNat", g("MyNat"))
        .expect("mzero registers");
    ctx.add_constructor_checked("msucc", "MyNat", arrow(g("MyNat"), g("MyNat")))
        .expect("msucc is strictly positive");

    // The recursive constructor's DOMAIN must be the inductive itself — the shape the
    // termination guard reads "smaller" off.
    let msucc_ty = ctx
        .get_constructors("MyNat")
        .into_iter()
        .find(|(n, _)| *n == "msucc")
        .map(|(_, t)| t.clone())
        .expect("msucc is registered");
    assert_eq!(
        nth_pi_domain(&msucc_ty, 0),
        Some(g("MyNat")),
        "msucc's domain must be MyNat itself, got {msucc_ty}"
    );

    // No prelude name was overwritten: `Nat`'s constructors are still `Zero`/`Succ`, in order.
    assert_eq!(
        ctor_names(&ctx, "Nat"),
        vec!["Zero", "Succ"],
        "the prelude's Nat/Zero/Succ are load-bearing for the numeral bridge"
    );

    // Structural recursion over the user type: `toNat : MyNat → Nat`, a `Fix` over a `Match`.
    let to_nat = Term::Fix {
        name: "f".to_string(),
        body: Box::new(lam(
            "m",
            g("MyNat"),
            match_(
                v("m"),
                lam("_", g("MyNat"), nat()),
                vec![g("Zero"), lam("k", g("MyNat"), succ(app(v("f"), v("k"))))],
            ),
        )),
    };
    let to_nat_ty = infer_type(&ctx, &to_nat).expect("structural recursion over MyNat typechecks");
    assert!(
        same_type(&ctx, &to_nat_ty, &arrow(g("MyNat"), nat())),
        "toNat : MyNat → Nat, got {to_nat_ty}"
    );
    assert_eq!(
        normalize(
            &ctx,
            &app(to_nat, app(g("msucc"), app(g("msucc"), g("mzero"))))
        ),
        succ(succ(g("Zero"))),
        "recursion on the user Peano type must compute"
    );

    // THE BRIDGE: a `Nat` literal is DEFINITIONALLY `Succ^n Zero` — but only for the prelude's
    // `Nat`, keyed on the literal strings `Zero`/`Succ`.
    let two = Term::Lit(logicaffeine_kernel::Literal::Nat(
        logicaffeine_kernel::BigInt::from_i64(2),
    ));
    assert!(
        defeq_for_test(&ctx, &two, &succ(succ(g("Zero")))),
        "the prelude's Nat literal bridge must make 2 ≡ Succ (Succ Zero)"
    );
    // …and NOT for a look-alike user type. Identifying the two would make `MyNat`- and
    // `Nat`-typed terms interchangeable, which they are not.
    assert!(
        !defeq_for_test(&ctx, &two, &app(g("msucc"), app(g("msucc"), g("mzero")))),
        "a user Peano type has no literal bridge — 2 must NOT be def-eq to msucc (msucc mzero)"
    );
}

// =============================================================================
// T03-003 · Parameterised inductive — the one shape where the arity default is right
// =============================================================================

#[test]
fn a_parameterised_list_is_all_parameters_and_no_indices() {
    let mut ctx = std_ctx();
    // `MyList (A : Type 0) : Type 0` — arity 1, and ALL of it is a uniform parameter.
    ctx.add_inductive("MyList", pi("A", ty0(), ty0()));
    let mylist = |a: Term| app(g("MyList"), a);
    ctx.add_constructor_checked("mnil", "MyList", pi("A", ty0(), mylist(v("A"))))
        .expect("mnil registers");
    ctx.add_constructor_checked(
        "mcons",
        "MyList",
        pi(
            "A",
            ty0(),
            arrow(v("A"), arrow(mylist(v("A")), mylist(v("A")))),
        ),
    )
    .expect("mcons is strictly positive and universe-consistent");

    assert_eq!(
        ctx.inductive_arity("MyList"),
        1,
        "MyList : Type → Type has arity 1"
    );
    assert_eq!(
        ctx.inductive_num_params("MyList"),
        1,
        "a uniform parameter must be counted as a PARAMETER"
    );
    assert_eq!(
        ctx.inductive_num_indices("MyList"),
        0,
        "a parameterised (non-indexed) inductive has zero indices"
    );

    // Every constructor carries the parameter telescope explicitly: `mcons` begins `Π(A:Type 0)`.
    let mcons_ty = ctx
        .get_constructors("MyList")
        .into_iter()
        .find(|(n, _)| *n == "mcons")
        .map(|(_, t)| t.clone())
        .expect("mcons is registered");
    assert_eq!(
        nth_pi_domain(&mcons_ty, 0),
        Some(ty0()),
        "a constructor must carry the inductive's parameter telescope, got {mcons_ty}"
    );

    // A `Match` whose parameters come from the discriminant: `length : MyList Nat → Nat`.
    let list_nat = mylist(nat());
    let length = Term::Fix {
        name: "f".to_string(),
        body: Box::new(lam(
            "xs",
            list_nat.clone(),
            match_(
                v("xs"),
                lam("_", list_nat.clone(), nat()),
                vec![
                    g("Zero"),
                    lam(
                        "h",
                        nat(),
                        lam("t", list_nat.clone(), succ(app(v("f"), v("t")))),
                    ),
                ],
            ),
        )),
    };
    let length_ty = infer_type(&ctx, &length).expect("a match on a parameterised list typechecks");
    assert!(
        same_type(&ctx, &length_ty, &arrow(list_nat.clone(), nat())),
        "length : MyList Nat → Nat, got {length_ty}"
    );
    let two_elems = apps(
        g("mcons"),
        &[
            nat(),
            g("Zero"),
            apps(g("mcons"), &[nat(), succ(g("Zero")), app(g("mnil"), nat())]),
        ],
    );
    assert_eq!(
        normalize(&ctx, &app(length, two_elems)),
        succ(succ(g("Zero"))),
        "the parameterised list must compute its own length"
    );

    // The same declaration serves every `Type 0` instantiation — the parameter is a term-level
    // argument of the inductive, not a universe (contrast T01-007).
    let bool_list = apps(
        g("mcons"),
        &[g("Bool"), g("true"), app(g("mnil"), g("Bool"))],
    );
    let bl_ty = infer_type(&ctx, &bool_list).expect("MyList Bool is the same declaration");
    assert!(
        same_type(&ctx, &bl_ty, &mylist(g("Bool"))),
        "MyList Bool, got {bl_ty}"
    );
}

// =============================================================================
// T03-004 · Indexed family — each constructor keeps its OWN index
// =============================================================================

#[test]
fn an_indexed_family_keeps_each_constructors_own_index() {
    let mut ctx = std_ctx();
    // `Tagged (A : Type 0) : Bool → Type 0` — 1 parameter, 1 index, and an index that is NOT a
    // `Nat`, so no numeral special-casing can rescue a wrong split.
    ctx.add_indexed_inductive("Tagged", pi("A", ty0(), arrow(g("Bool"), ty0())), 1);
    let tagged = |a: Term, b: Term| apps(g("Tagged"), &[a, b]);
    ctx.add_constructor(
        "plain",
        "Tagged",
        pi("A", ty0(), arrow(v("A"), tagged(v("A"), g("false")))),
    );
    ctx.add_constructor(
        "marked",
        "Tagged",
        pi("A", ty0(), arrow(v("A"), tagged(v("A"), g("true")))),
    );

    assert_eq!(ctx.inductive_num_params("Tagged"), 1, "A is the parameter");
    assert_eq!(
        ctx.inductive_num_indices("Tagged"),
        1,
        "the Bool is the index"
    );

    // THE FAMILY MUST NOT COLLAPSE: `plain`'s declared index is `false` and nothing may replace
    // it with the discriminant's. A flattened translation makes both inhabit every index.
    let p = apps(g("plain"), &[nat(), g("Zero")]);
    let p_ty = infer_type(&ctx, &p).expect("plain Nat Zero is well-typed");
    assert!(
        same_type(&ctx, &p_ty, &tagged(nat(), g("false"))),
        "plain's result index is `false`, got {p_ty}"
    );
    assert!(
        !is_subtype(&ctx, &p_ty, &tagged(nat(), g("true"))),
        "a `false`-indexed value must NOT inhabit the `true` index — that is the family collapsing"
    );

    // An indexed `Match`: the motive abstracts over the index AND the scrutinee, and the result
    // is the motive at the DISCRIMINANT's index.
    ctx.add_declaration("b", g("Bool"));
    ctx.add_declaration("t", tagged(nat(), g("b")));
    let motive = lam("i", g("Bool"), lam("z", tagged(nat(), v("i")), nat()));
    let untag = match_(
        g("t"),
        motive,
        vec![lam("a", nat(), g("Zero")), lam("a", nat(), succ(g("Zero")))],
    );
    let untag_ty = infer_type(&ctx, &untag).expect("an indexed match over Tagged must typecheck");
    assert!(
        same_type(&ctx, &untag_ty, &nat()),
        "the indexed match lands in Nat, got {untag_ty}"
    );
    match double_check(&ctx, &untag) {
        DoubleCheck::Agreed => {}
        other => panic!("both kernels must certify an indexed match, got {other:?}"),
    }

    // And the family's eliminator derives, with a motive that abstracts over the index.
    let (rec_ty, rec_term) = derive_recursor(&ctx, "Tagged").expect("Tagged's recursor derives");
    let motive_ty = nth_pi_domain(&rec_ty, 1).expect("the recursor takes a motive after A");
    assert!(
        count_leading_pis(&motive_ty) >= 2,
        "the motive must abstract over the index AND the scrutinee: {motive_ty}"
    );
    match double_check(&ctx, &rec_term) {
        DoubleCheck::Agreed => {}
        other => panic!("both kernels must certify the indexed recursor, got {other:?}"),
    }
}

// =============================================================================
// T03-006 · `mutual inductive` — one transactional block, never N separate ones
// =============================================================================

/// `A3 := a : B3 → A3`, `B3 := b : C3 → B3 | bstop`, `C3 := c : A3 → C3 | cstop` — a
/// THREE-member block, so the "take `num_params` from `block[0]`" rule has more than one
/// opportunity to disagree.
fn three_member_block() -> Vec<MutualInductive> {
    vec![
        MutualInductive {
            name: "A3".to_string(),
            sort: ty0(),
            num_params: 0,
            constructors: vec![("a3".to_string(), arrow(g("B3"), g("A3")))],
        },
        MutualInductive {
            name: "B3".to_string(),
            sort: ty0(),
            num_params: 0,
            constructors: vec![
                ("b3".to_string(), arrow(g("C3"), g("B3"))),
                ("bstop".to_string(), g("B3")),
            ],
        },
        MutualInductive {
            name: "C3".to_string(),
            sort: ty0(),
            num_params: 0,
            constructors: vec![
                ("c3".to_string(), arrow(g("A3"), g("C3"))),
                ("cstop".to_string(), g("C3")),
            ],
        },
    ]
}

#[test]
fn a_mutual_block_registers_transactionally_with_a_shared_recursor() {
    let mut ctx = std_ctx();
    ctx.add_mutual_inductives(&three_member_block())
        .expect("a positive three-member block must register in ONE call");

    // Block membership is what routes a sibling recursive occurrence to the sibling's fixpoint.
    // Registering the members one at a time leaves it empty and the recursion underivable.
    for m in ["A3", "B3", "C3"] {
        assert_eq!(
            ctx.mutual_block_of(m).map(|b| b.to_vec()),
            Some(vec!["A3".to_string(), "B3".to_string(), "C3".to_string()]),
            "every member must see the FULL ordered member list"
        );
        assert_eq!(
            ctx.inductive_num_params(m),
            0,
            "every member agrees on the parameter count"
        );
    }

    // Constructor order inside the block is Lean's, per member — `Match` is positional.
    assert_eq!(ctor_names(&ctx, "B3"), vec!["b3", "bstop"]);
    assert_eq!(ctor_names(&ctx, "C3"), vec!["c3", "cstop"]);

    // Every member gets a working eliminator, certified by BOTH kernels.
    for m in ["A3", "B3", "C3"] {
        let (ty, term) = derive_recursor(&ctx, m)
            .unwrap_or_else(|e| panic!("{m}'s mutual recursor derives: {e:?}"));
        let inferred =
            infer_type(&ctx, &term).unwrap_or_else(|e| panic!("{m}_rec typechecks: {e:?}"));
        assert!(
            same_type(&ctx, &inferred, &ty),
            "{m}_rec's reported type is the certified one"
        );
        match double_check(&ctx, &term) {
            DoubleCheck::Agreed => {}
            other => panic!("both kernels must certify {m}_rec, got {other:?}"),
        }
    }

    // A SHARED-PARAMETER block (`PTree A`/`PForest A`) is the other supported shape.
    let mut pctx = std_ctx();
    let ptree = |a: Term| app(g("PTree"), a);
    let pforest = |a: Term| app(g("PForest"), a);
    pctx.add_mutual_inductives(&[
        MutualInductive {
            name: "PTree".to_string(),
            sort: pi("A", ty0(), ty0()),
            num_params: 1,
            constructors: vec![(
                "pnode".to_string(),
                pi(
                    "A",
                    ty0(),
                    arrow(v("A"), arrow(pforest(v("A")), ptree(v("A")))),
                ),
            )],
        },
        MutualInductive {
            name: "PForest".to_string(),
            sort: pi("A", ty0(), ty0()),
            num_params: 1,
            constructors: vec![
                ("pnil".to_string(), pi("A", ty0(), pforest(v("A")))),
                (
                    "pcons".to_string(),
                    pi(
                        "A",
                        ty0(),
                        arrow(ptree(v("A")), arrow(pforest(v("A")), pforest(v("A")))),
                    ),
                ),
            ],
        },
    ])
    .expect("a mutual block with a shared uniform parameter must register");
    let (_, ptree_rec) = derive_recursor(&pctx, "PTree").expect("PTree's mutual recursor derives");
    match double_check(&pctx, &ptree_rec) {
        DoubleCheck::Agreed => {}
        other => {
            panic!("both kernels must certify the parameterised mutual recursor, got {other:?}")
        }
    }
}

// =============================================================================
// T03-007 · Mutual block with disagreeing parameter counts
// =============================================================================

/// Lean refuses `mutual inductive U (α)/inductive V end` outright ("number of parameters
/// mismatch in mutually inductive datatypes"). The shape LOGOS actually meets is the one below:
/// `Chain : Nat → Type` has arity 1 and `Tag : Type` arity 0, so the `add_inductive` default
/// ("all of the arity is parameters") assigns them 1 and 0.
fn mismatched_block() -> Vec<MutualInductive> {
    vec![
        MutualInductive {
            name: "Chain".to_string(),
            sort: arrow(nat(), ty0()),
            num_params: 1,
            constructors: vec![("cstop".to_string(), app(g("Chain"), g("Zero")))],
        },
        MutualInductive {
            name: "Tag".to_string(),
            sort: ty0(),
            num_params: 0,
            constructors: vec![("t1".to_string(), g("Tag"))],
        },
    ]
}

#[test]
fn a_mutual_block_whose_members_disagree_on_parameters_is_refused_at_registration() {
    let mut ctx = std_ctx();
    let registered = ctx.add_mutual_inductives(&mismatched_block());
    // What today's kernel says, and when it says it — surfaced in the failure message so the
    // work is named rather than merely asserted.
    let deferred = derive_recursor(&ctx, "Chain").err();

    assert!(
        registered.is_err(),
        "a mutual block whose members disagree on parameter count must be refused AT \
         REGISTRATION. `derive_mutual_recursor` takes the shared telescope from block[0] and \
         checks the agreement there, which is far too late: the block is already in the context \
         and can never be eliminated. Deferred failure observed: {deferred:?}"
    );
    assert!(
        !ctx.is_inductive("Chain")
            && !ctx.is_inductive("Tag")
            && ctx.mutual_block_of("Chain").is_none()
            && ctx.get_global("cstop").is_none(),
        "a refused block must leave NOTHING behind — registration is transactional or it is a trap"
    );
}

// =============================================================================
// T03-008 · Nested inductive through a pure unary container
// =============================================================================

/// `Labelled := lnode : Nat → TList Labelled → Labelled` — a nested occurrence alongside an
/// ORDINARY field, which must pass through unspecialised.
fn labelled_decl() -> NestedDecl {
    NestedDecl {
        name: "Labelled".to_string(),
        sort: ty0(),
        constructors: vec![(
            "lnode".to_string(),
            arrow(nat(), arrow(app(g("TList"), g("Labelled")), g("Labelled"))),
        )],
    }
}

#[test]
fn a_nested_inductive_through_a_unary_container_compiles_to_a_checked_block() {
    let mut ctx = std_ctx();
    let info = ctx
        .add_nested_inductive(&labelled_decl())
        .expect("a TList nesting compiles");

    assert_eq!(
        info.siblings,
        vec!["Labelled$TList".to_string()],
        "the container is specialised into exactly one mutual sibling"
    );
    assert_eq!(
        ctx.mutual_block_of("Labelled").map(|b| b.to_vec()),
        Some(vec!["Labelled".to_string(), "Labelled$TList".to_string()]),
        "the nested type and its sibling form one mutual block"
    );

    // The ORDINARY `Nat` field survives; only the nested argument is specialised.
    let lnode_ty = infer_type(&ctx, &g("lnode")).expect("lnode is registered");
    assert!(
        same_type(
            &ctx,
            &lnode_ty,
            &arrow(nat(), arrow(g("Labelled$TList"), g("Labelled")))
        ),
        "lnode : Nat → Labelled$TList → Labelled, got {lnode_ty}"
    );

    // The isos are the bridge to the generic container, and they are KERNEL-CHECKED — the
    // compiler itself is untrusted, so soundness rides on these.
    let iso = &info.isos[0];
    let to_ty =
        infer_type(&ctx, &g(&iso.to_generic)).expect("the to-iso is a registered definition");
    let from_ty =
        infer_type(&ctx, &g(&iso.from_generic)).expect("the from-iso is a registered definition");
    assert!(
        same_type(
            &ctx,
            &to_ty,
            &arrow(g("Labelled$TList"), app(g("TList"), g("Labelled")))
        ),
        "to_generic : Labelled$TList → TList Labelled, got {to_ty}"
    );
    assert!(
        same_type(
            &ctx,
            &from_ty,
            &arrow(app(g("TList"), g("Labelled")), g("Labelled$TList"))
        ),
        "from_generic : TList Labelled → Labelled$TList, got {from_ty}"
    );
    for name in [&iso.to_generic, &iso.from_generic] {
        match double_check(&ctx, &g(name)) {
            DoubleCheck::Agreed => {}
            other => panic!("both kernels must certify iso {name}, got {other:?}"),
        }
    }

    // The payoff: the nested type recurses THROUGH the container.
    let (_, rec_term) = derive_recursor(&ctx, "Labelled").expect("Labelled's recursor derives");
    match double_check(&ctx, &rec_term) {
        DoubleCheck::Agreed => {}
        other => panic!("both kernels must certify Labelled_rec, got {other:?}"),
    }

    // A declaration with no genuine nesting is refused outright — `add_nested_inductive` is not
    // a general registration path.
    let mut plain = std_ctx();
    let not_nested = NestedDecl {
        name: "WithNats".to_string(),
        sort: ty0(),
        constructors: vec![(
            "wn".to_string(),
            arrow(app(g("TList"), nat()), g("WithNats")),
        )],
    };
    assert!(
        plain.add_nested_inductive(&not_nested).is_err(),
        "`TList Nat` does not mention the inductive — it must be registered directly, not nested"
    );
}

// =============================================================================
// T03-009 · Nesting outside the specialisable fragment
// =============================================================================

/// A BINARY container. `analyze_container_ctor` binds `params[0]` as *the* element and every
/// later argument is classified against it, so the second type parameter is `ArgKind::Other`
/// and no binary container is ever specialisable.
fn register_tpair(ctx: &mut Context) {
    ctx.add_inductive("TPair", pi("A", ty0(), pi("B", ty0(), ty0())));
    ctx.add_constructor(
        "tpair",
        "TPair",
        pi(
            "A",
            ty0(),
            pi(
                "B",
                ty0(),
                arrow(v("A"), arrow(v("B"), apps(g("TPair"), &[v("A"), v("B")]))),
            ),
        ),
    );
}

#[test]
fn nesting_through_a_binary_container_is_registrable() {
    let mut ctx = std_ctx();
    register_tpair(&mut ctx);
    // `PTree := pnode : TPair Nat PTree → PTree` — Lean accepts it and compiles it to a mutual
    // block; LOGOS's specialiser only handles pure unary containers.
    let decl = NestedDecl {
        name: "PTree".to_string(),
        sort: ty0(),
        constructors: vec![(
            "pnode".to_string(),
            arrow(apps(g("TPair"), &[nat(), g("PTree")]), g("PTree")),
        )],
    };
    let outcome = ctx.add_nested_inductive(&decl);

    // FAIL-CLOSED, whatever we decide about the capability: a refusal must leave nothing behind.
    // The block and the isos are emitted together, and half of them is worse than none.
    if outcome.is_err() {
        assert!(
            !ctx.is_inductive("PTree")
                && !ctx.is_inductive("PTree$TPair")
                && ctx.get_global("pnode").is_none()
                && ctx.mutual_block_of("PTree").is_none(),
            "a refused nesting must register no member, no sibling and no iso"
        );
    }

    assert!(
        outcome.is_ok(),
        "Lean nests an inductive inside a binary container (`List (Nat × T)`, `Array T`, a \
         labelled container) and compiles it to a mutual block. LOGOS's specialiser accepts only \
         PURE unary containers — every argument the bare element or a recursive occurrence — so \
         this mathematics has no LOGOS declaration at all: {outcome:?}"
    );
}

// =============================================================================
// T03-010 · `structure` — one call for the inductive, the constructor, N projections, eta
// =============================================================================

#[test]
fn a_structure_yields_a_constructor_ordered_projections_and_eta() {
    let mut ctx = std_ctx();
    // Three fields, two of which share a type — so a permuted field order is type-invisible
    // at the constructor and only the computed projection catches it.
    ctx.add_structure(
        "Triple",
        &[],
        &[("a", nat()), ("b", g("Bool")), ("c", nat())],
    );

    let info = ctx
        .struct_info("Triple")
        .expect("add_structure must register StructInfo")
        .clone();
    assert_eq!(info.mk, "Triple_mk", "the constructor is `{{name}}_mk`");
    assert_eq!(info.num_params, 0, "Triple takes no parameters");
    assert_eq!(
        info.projections,
        vec![
            "Triple_a".to_string(),
            "Triple_b".to_string(),
            "Triple_c".to_string()
        ],
        "projections are in FIELD order — a permutation makes p.a return p.c silently"
    );

    // Each projection computes on a literal constructor.
    let t = apps(g("Triple_mk"), &[g("Zero"), g("true"), succ(g("Zero"))]);
    assert_eq!(
        normalize(&ctx, &app(g("Triple_a"), t.clone())),
        g("Zero"),
        "p.a is the FIRST field"
    );
    assert_eq!(
        normalize(&ctx, &app(g("Triple_b"), t.clone())),
        g("true"),
        "p.b is the second field"
    );
    assert_eq!(
        normalize(&ctx, &app(g("Triple_c"), t)),
        succ(g("Zero")),
        "p.c is the third field — `a` and `c` share a type, so only the VALUE separates them"
    );

    // DEFINITIONAL ETA — the property only the `structures` registry grants.
    ctx.add_declaration("p", g("Triple"));
    let expanded = apps(
        g("Triple_mk"),
        &[
            app(g("Triple_a"), g("p")),
            app(g("Triple_b"), g("p")),
            app(g("Triple_c"), g("p")),
        ],
    );
    assert!(
        defeq_for_test(&ctx, &expanded, &g("p")),
        "structure eta: ⟨p.a, p.b, p.c⟩ ≡ p"
    );

    // DEPENDENT fields: a later field's type may reference an earlier one, and `add_structure`
    // rewrites that reference into the earlier field's PROJECTION of the discriminant.
    ctx.add_structure(
        "Sized",
        &[],
        &[
            ("bound", nat()),
            ("wit", apps(g("Eq"), &[nat(), v("bound"), v("bound")])),
        ],
    );
    let wit_ty = infer_type(&ctx, &g("Sized_wit")).expect("the dependent projection is registered");
    let want = pi(
        "s",
        g("Sized"),
        apps(
            g("Eq"),
            &[
                nat(),
                app(g("Sized_bound"), v("s")),
                app(g("Sized_bound"), v("s")),
            ],
        ),
    );
    assert!(
        same_type(&ctx, &wit_ty, &want),
        "a dependent field's projection type must mention Sized_bound s, got {wit_ty}"
    );

    // NEGATIVE CONTROL: a one-constructor INDUCTIVE that was not declared as a structure gets
    // no eta — Lean does not grant it either.
    ctx.add_inductive("Wrapper", ty0());
    ctx.add_constructor_checked("wrap", "Wrapper", arrow(nat(), g("Wrapper")))
        .expect("wrap registers");
    ctx.add_definition(
        "unwrap".to_string(),
        arrow(g("Wrapper"), nat()),
        lam(
            "w",
            g("Wrapper"),
            match_(
                v("w"),
                lam("_", g("Wrapper"), nat()),
                vec![lam("n", nat(), v("n"))],
            ),
        ),
    );
    ctx.add_declaration("w", g("Wrapper"));
    assert!(
        ctx.struct_info("Wrapper").is_none(),
        "a plain inductive is not in the struct registry"
    );
    assert!(
        !defeq_for_test(&ctx, &app(g("wrap"), app(g("unwrap"), g("w"))), &g("w")),
        "eta must NOT fire for a one-constructor inductive that was not declared a structure"
    );
}

// =============================================================================
// T03-011 · Structure projection — DIVERGE
// =============================================================================

/// **DIVERGE.** Lean's `Expr.proj` is a kernel primitive with its own iota rule. LOGOS has no
/// `Term::Proj` at all: a projection is an ordinary global DEFINITION whose body is a
/// `Term::Match`, applied to the structure's parameters first and the receiver last. That is a
/// strictly smaller trusted base — the projection is a certified term the second kernel
/// re-checks like any other, rather than a primitive reduction rule the TCB must be trusted on —
/// and it costs one delta step. The other half of the corpus's `@loss` is not a divergence at
/// all: structure eta living in `def_eq` and not in `normalize` is exactly where Lean puts it
/// too (`isDefEq`, not `whnf`), and every conversion the kernel performs goes through `def_eq`.
/// This test pins that boundary precisely in both directions.
#[test]
fn a_projection_is_a_parameters_first_global_and_eta_lives_in_conversion() {
    let mut ctx = std_ctx();
    ctx.add_structure(
        "Pair",
        &[("A", ty0()), ("B", ty0())],
        &[("fst", v("A")), ("snd", v("B"))],
    );

    // PARAMETERS FIRST: `p.fst` is `Pair_fst A B p`, never `Pair_fst p`.
    let fst_ty = infer_type(&ctx, &g("Pair_fst")).expect("Pair_fst is a registered definition");
    let want = pi(
        "A",
        ty0(),
        pi(
            "B",
            ty0(),
            pi("s", apps(g("Pair"), &[v("A"), v("B")]), v("A")),
        ),
    );
    assert!(
        same_type(&ctx, &fst_ty, &want),
        "Pair_fst : Π(A).Π(B).Π(s : Pair A B). A — parameters before the receiver, got {fst_ty}"
    );
    assert!(
        ctx.is_definition("Pair_fst"),
        "a projection is a DEFINITION, not an axiom"
    );

    // Applying the receiver first is simply ill-typed — the arity is not negotiable.
    ctx.add_declaration("p", apps(g("Pair"), &[nat(), g("Bool")]));
    assert!(
        infer_type(&ctx, &app(g("Pair_fst"), g("p"))).is_err(),
        "Pair_fst p (receiver first) must not typecheck"
    );

    // It COMPUTES on a literal constructor — delta then iota, two steps rather than Lean's one.
    let pair = apps(g("Pair_mk"), &[nat(), g("Bool"), g("Zero"), g("true")]);
    assert_eq!(
        normalize(
            &ctx,
            &apps(g("Pair_fst"), &[nat(), g("Bool"), pair.clone()])
        ),
        g("Zero"),
        "⟨a, b⟩.fst must reduce to a"
    );
    assert_eq!(
        normalize(&ctx, &apps(g("Pair_snd"), &[nat(), g("Bool"), pair])),
        g("true"),
        "⟨a, b⟩.snd must reduce to b"
    );

    // ETA IS A CONVERSION RULE — it decides `⟨p.fst, p.snd⟩ ≡ p`, and the SECOND kernel agrees.
    let expanded = apps(
        g("Pair_mk"),
        &[
            nat(),
            g("Bool"),
            apps(g("Pair_fst"), &[nat(), g("Bool"), g("p")]),
            apps(g("Pair_snd"), &[nat(), g("Bool"), g("p")]),
        ],
    );
    assert!(
        defeq_for_test(&ctx, &expanded, &g("p")),
        "def_eq must decide ⟨p.fst, p.snd⟩ ≡ p — this is what Lean's `rfl` uses"
    );
    // A term whose TYPING requires eta: `refl _ p : Eq _ p p` checked against
    // `Eq _ p ⟨p.fst, p.snd⟩`. Without the eta rule the third argument does not convert and the
    // application is ill-typed — so this is not merely a `defeq_for_test` restated.
    let pair_nb = apps(g("Pair"), &[nat(), g("Bool")]);
    let eq_ty = apps(g("Eq"), &[pair_nb.clone(), g("p"), expanded.clone()]);
    let forced = app(
        lam("h", eq_ty.clone(), v("h")),
        apps(g("refl"), &[pair_nb, g("p")]),
    );
    let forced_ty = infer_type(&ctx, &forced)
        .unwrap_or_else(|e| panic!("a term whose typing needs structure eta must check: {e:?}"));
    assert!(
        same_type(&ctx, &forced_ty, &eq_ty),
        "the eta-requiring application lands at the stated equation, got {forced_ty}"
    );
    match double_check(&ctx, &forced) {
        DoubleCheck::Agreed => {}
        other => panic!("the independent re-checker must agree on structure eta, got {other:?}"),
    }

    // …AND IT IS NOT A REDUCTION. `normalize` does not eta-expand a neutral receiver, exactly as
    // Lean's `whnf` does not. Conversion is where eta belongs; a normalizer that expanded here
    // would never reach a normal form.
    assert_ne!(
        normalize(&ctx, &expanded),
        normalize(&ctx, &g("p")),
        "structure eta is a CONVERSION rule, not a reduction — normalize must leave both alone \
         and `def_eq` must still identify them"
    );
}

// =============================================================================
// T03-012 · Structure in `Prop` or `Type u` (u ≥ 1)
// =============================================================================

#[test]
fn a_structure_can_live_in_prop_or_above_type_zero() {
    let mut ctx = std_ctx();

    // CONTROL: the CHECKED constructor path does catch the universe violation that
    // `add_structure` never runs — a `Type 0` inductive may not store a `Type 0`-typed field.
    ctx.add_inductive("Carrier", ty0());
    let carrier_mk = pi("ty", ty0(), arrow(v("ty"), g("Carrier")));
    assert!(
        matches!(
            ctx.add_constructor_checked("Carrier_mk0", "Carrier", carrier_mk.clone()),
            // A decided semantic refusal, not incomplete checking (constructor_field_universe_verdicts).
            Err(KernelError::InvalidDeclaration(ref m)) if m.contains("universe inconsistency")
        ),
        "a Type 0 inductive storing a Type 0-typed field is a universe inconsistency"
    );

    // THE GAP, half one: a `Prop`-valued record. `add_structure`'s codomain is a literal
    // `Sort(Type 0)` and its signature has nowhere to pass a sort, so `Conj P Q : Prop` — the
    // shape of `And`, `Iff` and every bundled `Prop`-valued class — is unreachable. Re-sorting
    // it to `Type 0` turns proofs into data: proof irrelevance stops applying and the
    // large-elimination gate stops firing.
    ctx.add_structure(
        "Conj",
        &[("P", prop()), ("Q", prop())],
        &[("left", v("P")), ("right", v("Q"))],
    );
    let conj_sort = ctx.get_global("Conj").cloned().expect("Conj is registered");
    let mut codomain = &conj_sort;
    while let Term::Pi { body_type, .. } = codomain {
        codomain = body_type;
    }
    assert_eq!(
        codomain,
        &prop(),
        "a structure declared in Prop must be REGISTERED in Prop; add_structure hard-codes \
         Sort(Type 0), so its inhabitants become data. Got the whole arity {conj_sort}"
    );

    // THE GAP, half two: a record that genuinely lives in `Type 1` because it stores a `Type`.
    // `add_structure` registers it at `Type 0` AND routes its constructor through the unchecked
    // `add_constructor`, so the violation the control above catches is never even tested.
    ctx.add_structure("Bundle", &[], &[("ty", ty0()), ("point", v("ty"))]);
    assert_eq!(
        ctx.get_global("Bundle"),
        Some(&ty1()),
        "a record storing a Type-valued field lives in Type 1; registering it at Type 0 is the \
         Girard/Hurkens door, and add_structure never universe-checks its fields"
    );
}

// =============================================================================
// T03-013 · `structure … extends`
// =============================================================================

#[test]
fn a_child_structure_inherits_its_parents_fields_and_gets_a_parent_coercion() {
    let mut ctx = std_ctx();
    ctx.add_structure("Base", &[], &[("x", nat()), ("y", nat())]);
    // The flattened child: the parent's fields first, in parent order, then the child's own.
    ctx.add_structure("Extended", &[], &[("x", nat()), ("y", nat()), ("z", nat())]);

    let info = ctx
        .struct_info("Extended")
        .expect("Extended is a structure")
        .clone();
    assert_eq!(
        &info.projections[..2],
        &["Extended_x".to_string(), "Extended_y".to_string()],
        "a flattened child's projections must BEGIN with the parent's fields in parent order"
    );

    // NO SUBTYPING between two inductives: the parent's projection does not apply to a child.
    // Lean inserts `Extended.toBase`; LOGOS has no coercion at the kernel level at all.
    ctx.add_declaration("c", g("Extended"));
    assert!(
        infer_type(&ctx, &app(g("Base_x"), g("c"))).is_err(),
        "Base_x must NOT accept an Extended — there is no structural subtyping between inductives"
    );

    // CONTROL: the conversion is CONSTRUCTIBLE, and as a definition (not an axiom) it satisfies
    // the round-trip law definitionally.
    let mut hand = ctx.clone();
    hand.add_definition(
        "Extended_toBase".to_string(),
        arrow(g("Extended"), g("Base")),
        lam(
            "e",
            g("Extended"),
            apps(
                g("Base_mk"),
                &[app(g("Extended_x"), v("e")), app(g("Extended_y"), v("e"))],
            ),
        ),
    );
    assert_eq!(
        normalize(&hand, &app(g("Base_x"), app(g("Extended_toBase"), g("c")))),
        normalize(&hand, &app(g("Extended_x"), g("c"))),
        "(c.toBase).x ≡ c.x — the law that pins toBase as a definition rather than an axiom"
    );
    assert_eq!(
        normalize(&hand, &app(g("Base_y"), app(g("Extended_toBase"), g("c")))),
        normalize(&hand, &app(g("Extended_y"), g("c"))),
        "(c.toBase).y ≡ c.y"
    );

    // THE GAP: declaring the child does not record the inheritance or generate the conversion.
    // `StructInfo` is `{ mk, num_params, projections }` — three fields, none of which is a
    // parent — so nothing downstream can be checked against the relationship, and a diamond's
    // two routes to a shared parent are never proved to agree.
    assert!(
        ctx.get_global("Extended_toBase").is_some(),
        "a child structure must come with its parent conversion. Today `add_structure` takes a \
         flat field slice with no parent list, so `extends` has to be hand-flattened AND the \
         coercion hand-written at every declaration"
    );
}

// =============================================================================
// T03-014 · `class` as a structure
// =============================================================================

#[test]
fn a_class_is_a_structure_whose_methods_are_projections_found_by_instance_search() {
    let mut ctx = std_ctx();
    // The carrier is an ordinary record — `add_structure`, so eta fires and an instance really
    // is determined by its methods.
    ctx.add_structure("Pointed", &[("A", ty0())], &[("point", v("A"))]);
    let info = ctx
        .struct_info("Pointed")
        .expect("the class carrier is a structure")
        .clone();
    assert_eq!(
        info.projections,
        vec!["Pointed_point".to_string()],
        "a method IS a projection"
    );

    // Instances are entries in a flat list, searched by unifying against the required type.
    ctx.add_instance(
        app(g("Pointed"), nat()),
        apps(g("Pointed_mk"), &[nat(), g("Zero")]),
    );
    ctx.add_instance(
        app(g("Pointed"), g("Bool")),
        apps(g("Pointed_mk"), &[g("Bool"), g("true")]),
    );

    let mut mctx = MetaCtx::new();
    let inst = resolve_instance(&ctx, &mut mctx, &app(g("Pointed"), nat()))
        .expect("instance search must find `Pointed Nat`");
    let inst_ty = infer_type(&ctx, &inst).expect("the resolved instance is well-typed");
    assert!(
        same_type(&ctx, &inst_ty, &app(g("Pointed"), nat())),
        "resolution must return an instance of the REQUIRED class type, got {inst_ty}"
    );
    assert_eq!(
        normalize(&ctx, &apps(g("Pointed_point"), &[nat(), inst.clone()])),
        g("Zero"),
        "a method call is the projection applied to the resolved instance"
    );

    let mut mctx2 = MetaCtx::new();
    let binst = resolve_instance(&ctx, &mut mctx2, &app(g("Pointed"), g("Bool")))
        .expect("instance search must distinguish `Pointed Bool` from `Pointed Nat`");
    assert_eq!(
        normalize(&ctx, &apps(g("Pointed_point"), &[g("Bool"), binst])),
        g("true"),
        "the Bool instance must not be answered by the Nat one"
    );

    // Two methods of the SAME type, so the projection order is observable.
    ctx.add_structure(
        "Measured",
        &[("A", ty0())],
        &[
            ("weight", arrow(v("A"), nat())),
            ("height", arrow(v("A"), nat())),
        ],
    );
    let m = ctx
        .struct_info("Measured")
        .expect("Measured is a structure")
        .clone();
    assert_eq!(
        m.projections,
        vec!["Measured_weight".to_string(), "Measured_height".to_string()],
        "method order is declaration order — both return Nat, so a permutation is type-invisible"
    );
    let minst = apps(
        g("Measured_mk"),
        &[
            g("Bool"),
            lam("b", g("Bool"), succ(g("Zero"))),
            lam("b", g("Bool"), g("Zero")),
        ],
    );
    assert_eq!(
        normalize(
            &ctx,
            &app(
                apps(g("Measured_weight"), &[g("Bool"), minst.clone()]),
                g("true")
            )
        ),
        succ(g("Zero")),
        "weight is the FIRST method"
    );
    assert_eq!(
        normalize(
            &ctx,
            &app(apps(g("Measured_height"), &[g("Bool"), minst]), g("true"))
        ),
        g("Zero"),
        "height is the second method"
    );

    // An INTERLEAVED instance-implicit binder — `f (A : Type) [C A] (n : Nat)` has kinds
    // [Explicit, Instance, Explicit], which the "N leading implicits" count model cannot express.
    ctx.add_definition(
        "repeatPoint".to_string(),
        pi(
            "A",
            ty0(),
            arrow(app(g("Pointed"), v("A")), arrow(nat(), nat())),
        ),
        lam(
            "A",
            ty0(),
            lam("i", app(g("Pointed"), v("A")), lam("n", nat(), v("n"))),
        ),
    );
    ctx.set_binder_kinds(
        "repeatPoint",
        vec![
            ParamKind::Explicit,
            ParamKind::Instance,
            ParamKind::Explicit,
        ],
    );
    assert_eq!(
        ctx.binder_kinds("repeatPoint"),
        Some(&[ParamKind::Explicit, ParamKind::Instance, ParamKind::Explicit][..]),
        "an instance binder's POSITION must survive registration — Instance is resolved after \
         the explicit arguments, Implicit is solved immediately, and the two are not interchangeable"
    );

    // Class eta: an instance is determined by its methods. This holds only because the carrier
    // went through `add_structure`.
    ctx.add_declaration("i", app(g("Pointed"), nat()));
    assert!(
        defeq_for_test(
            &ctx,
            &apps(
                g("Pointed_mk"),
                &[nat(), apps(g("Pointed_point"), &[nat(), g("i")])]
            ),
            &g("i")
        ),
        "⟨C.method inst⟩ ≡ inst — the eta a hand-assembled carrier would lose"
    );
}

// =============================================================================
// T03-015 · Strict positivity — the paradox fence
// =============================================================================

#[test]
fn a_negative_occurrence_is_refused_and_its_positive_neighbours_are_not() {
    let mut ctx = std_ctx();

    // `Bad := mk : (Bad → False) → Bad` — the classical paradox. The CHECKED path refuses it…
    ctx.add_inductive("Bad", ty0());
    let bad_mk = arrow(arrow(g("Bad"), g("False")), g("Bad"));
    assert!(
        matches!(
            ctx.add_constructor_checked("bad_mk", "Bad", bad_mk.clone()),
            Err(KernelError::PositivityViolation { .. })
        ),
        "a negative occurrence must be a PositivityViolation"
    );
    // …and registers nothing.
    assert!(
        ctx.get_global("bad_mk").is_none() && ctx.get_constructors("Bad").is_empty(),
        "a refused constructor must leave no trace"
    );
    // The check is available standalone, so a translator can re-run it on the term it emitted
    // rather than trusting that Lean checked the *Lean* declaration.
    assert!(
        check_positivity_for_test("Bad", "bad_mk", &bad_mk).is_err(),
        "check_positivity must refuse the paradox on its own"
    );

    // ONE ARROW APART — the strictly-positive neighbours must all be ACCEPTED. A too-eager
    // check is as wrong as a missing one.
    ctx.add_inductive("GoodFn", ty0());
    ctx.add_constructor_checked("gstop", "GoodFn", g("GoodFn"))
        .expect("gstop registers");
    ctx.add_constructor_checked(
        "gmk",
        "GoodFn",
        arrow(arrow(nat(), g("GoodFn")), g("GoodFn")),
    )
    .expect("(Nat → GoodFn) → GoodFn is strictly positive");

    ctx.add_inductive("GoodNested", ty0());
    ctx.add_constructor_checked("nstop", "GoodNested", g("GoodNested"))
        .expect("nstop registers");
    ctx.add_constructor_checked(
        "nmk",
        "GoodNested",
        arrow(arrow(arrow(nat(), nat()), g("GoodNested")), g("GoodNested")),
    )
    .expect("((Nat → Nat) → GoodNested) → GoodNested is strictly positive");

    register_tree(&mut ctx);
    ctx.add_inductive("UsesOther", ty0());
    ctx.add_constructor_checked(
        "umk",
        "UsesOther",
        arrow(arrow(g("Tree"), nat()), g("UsesOther")),
    )
    .expect("a DIFFERENT inductive in a domain is not a positivity violation");

    // CROSS-BLOCK: `Ping := p : (Pong → False) → Ping`, `Pong := q : Ping → Pong`. Positivity is
    // checked over the WHOLE block, before anything is mutated.
    let mut mctx = std_ctx();
    let paradox_block = vec![
        MutualInductive {
            name: "Ping".to_string(),
            sort: ty0(),
            num_params: 0,
            constructors: vec![(
                "pmk".to_string(),
                arrow(arrow(g("Pong"), g("False")), g("Ping")),
            )],
        },
        MutualInductive {
            name: "Pong".to_string(),
            sort: ty0(),
            num_params: 0,
            constructors: vec![("qmk".to_string(), arrow(g("Ping"), g("Pong")))],
        },
    ];
    assert!(
        matches!(
            mctx.add_mutual_inductives(&paradox_block),
            Err(KernelError::PositivityViolation { .. })
        ),
        "a sibling in a negative position is a cross-block paradox"
    );
    assert!(
        !mctx.is_inductive("Ping")
            && !mctx.is_inductive("Pong")
            && mctx.get_global("pmk").is_none()
            && mctx.get_global("qmk").is_none(),
        "the mutual registration is TRANSACTIONAL — a positivity failure adds nothing"
    );

    // WHY the mutual API exists: the single-inductive check is scoped to ONE name by design, so
    // a SIBLING in a negative position is invisible to it. Routing a block member through
    // `add_constructor_checked` is how the cross-block paradox gets in.
    assert!(
        check_positivity_for_test("Ping", "pmk", &arrow(arrow(g("Pong"), g("False")), g("Ping")))
            .is_ok(),
        "check_positivity is scoped to one inductive — `Pong` is not `Ping`, so only \
         check_positivity_mutual can catch this. Mutual blocks MUST go through add_mutual_inductives"
    );

    // The positive mutual neighbour is accepted.
    let mut okctx = std_ctx();
    assert!(
        okctx
            .add_mutual_inductives(&[
                MutualInductive {
                    name: "PingOk".to_string(),
                    sort: ty0(),
                    num_params: 0,
                    constructors: vec![
                        ("pok".to_string(), arrow(g("PongOk"), g("PingOk"))),
                        ("pstop".to_string(), g("PingOk")),
                    ],
                },
                MutualInductive {
                    name: "PongOk".to_string(),
                    sort: ty0(),
                    num_params: 0,
                    constructors: vec![
                        ("qok".to_string(), arrow(g("PingOk"), g("PongOk"))),
                        ("qstop".to_string(), g("PongOk")),
                    ],
                },
            ])
            .is_ok(),
        "flipping the occurrence out of the domain makes the same block legal"
    );
}

// =============================================================================
// T03-016 · Inductive propositions
// =============================================================================

#[test]
fn the_prelude_propositions_keep_their_order_and_their_elimination_gate() {
    let mut ctx = std_ctx();

    // Map onto the prelude, never re-declare: these names are load-bearing for `decide`,
    // `Decidable_rec` and the auto certifier.
    assert_eq!(
        ctor_names(&ctx, "And"),
        vec!["conj"],
        "And's single constructor"
    );
    assert_eq!(
        ctor_names(&ctx, "Or"),
        vec!["left", "right"],
        "Or's constructor ORDER — both branches of a disjunction elimination have the same type \
         whenever the motive is constant, so a swapped mapping is type-invisible"
    );
    assert_eq!(
        ctor_names(&ctx, "Ex"),
        vec!["witness"],
        "Ex's single constructor"
    );
    assert_eq!(
        ctor_names(&ctx, "True"),
        vec!["I"],
        "True's single constructor"
    );
    assert!(
        ctx.get_constructors("False").is_empty(),
        "False has NO constructors"
    );
    for p in ["And", "Or", "Ex", "True", "False"] {
        assert_eq!(
            ctx.get_global(p).map(|t| {
                let mut c = t;
                while let Term::Pi { body_type, .. } = c {
                    c = body_type;
                }
                c.clone()
            }),
            Some(prop()),
            "{p} must live in Prop — Prop is a SEPARATE Universe constructor, not Type 0"
        );
    }

    ctx.add_declaration("P", prop());
    ctx.add_declaration("Q", prop());
    ctx.add_declaration("hand", apps(g("And"), &[g("P"), g("Q")]));
    ctx.add_declaration("hor", apps(g("Or"), &[g("P"), g("Q")]));

    // `And` is a SUBSINGLETON (one constructor, both arguments proofs) so it large-eliminates.
    let and_to_nat = match_(
        g("hand"),
        lam("_", apps(g("And"), &[g("P"), g("Q")]), nat()),
        vec![lam("p", g("P"), lam("q", g("Q"), g("Zero")))],
    );
    assert!(
        infer_type(&ctx, &and_to_nat).is_ok(),
        "And is a subsingleton — eliminating it into Type must be allowed (this is Eq/False/Acc's \
         rule too)"
    );

    // `Or` is NOT, so large elimination must be REFUSED — a proof may not pick a Type-level value.
    let or_to_nat = match_(
        g("hor"),
        lam("_", apps(g("Or"), &[g("P"), g("Q")]), nat()),
        vec![
            lam("p", g("P"), g("Zero")),
            lam("q", g("Q"), succ(g("Zero"))),
        ],
    );
    assert!(
        matches!(
            infer_type(&ctx, &or_to_nat),
            Err(KernelError::InvalidMotive(_))
        ),
        "large-eliminating Or must be refused: {:?}",
        infer_type(&ctx, &or_to_nat)
    );

    // …while eliminating `Or` into `Prop` is always legal, for any number of constructors.
    let or_to_prop = match_(
        g("hor"),
        lam("_", apps(g("Or"), &[g("P"), g("Q")]), g("True")),
        vec![lam("p", g("P"), g("I")), lam("q", g("Q"), g("I"))],
    );
    assert!(
        infer_type(&ctx, &or_to_prop).is_ok(),
        "Prop → Prop elimination is never gated"
    );

    // A USER `Prop` inductive registers at `Prop` through the checked path, and impredicativity
    // exempts it from the constructor universe constraint entirely.
    ctx.add_inductive("AllProps", prop());
    ctx.add_constructor_checked(
        "apmk",
        "AllProps",
        arrow(pi("R", prop(), arrow(v("R"), v("R"))), g("AllProps")),
    )
    .expect("impredicative Prop admits a constructor quantifying over all propositions");
    assert_eq!(
        ctx.get_global("AllProps"),
        Some(&prop()),
        "the user proposition stays in Prop"
    );

    // A user `Prop` inductive with INDICES — the shape the prelude uses for `Eq` and `Acc`.
    ctx.add_indexed_inductive("Positive", arrow(nat(), prop()), 0);
    ctx.add_constructor("pone", "Positive", app(g("Positive"), succ(g("Zero"))));
    ctx.add_constructor(
        "pmore",
        "Positive",
        pi(
            "n",
            nat(),
            arrow(app(g("Positive"), v("n")), app(g("Positive"), succ(v("n")))),
        ),
    );
    assert_eq!(ctx.inductive_num_params("Positive"), 0, "zero parameters");
    assert_eq!(ctx.inductive_num_indices("Positive"), 1, "one index");
    // `pone : Positive 1`, so `pmore 1 pone : Positive 2` and `pmore 2 that : Positive 3` — the
    // index that each constructor DECLARES is what forces the chain.
    let two = apps(g("pmore"), &[succ(g("Zero")), g("pone")]);
    let three = apps(g("pmore"), &[succ(succ(g("Zero"))), two]);
    let three_ty = infer_type(&ctx, &three).expect("the indexed proposition is inhabited at 3");
    assert!(
        same_type(
            &ctx,
            &three_ty,
            &app(g("Positive"), succ(succ(succ(g("Zero")))))
        ),
        "Positive 3, got {three_ty}"
    );
}

// =============================================================================
// T03-017 · The large-elimination gate
// =============================================================================

/// The corpus's `@guard` about `is_subsingleton_prop`'s ARITY OFFSET — the positional skip that
/// makes LOGOS stricter than Lean on one constructor shape and MORE PERMISSIVE on another — is
/// confirmed defect 2, reproduced by
/// `lean_conformance_defects::large_elimination_admits_exactly_the_propositional_and_indexed_constructor_fields`
/// (claimed by T01-014). This test decides the rest of T03-017's oracle: that the gate exists,
/// that it admits exactly the canonical subsingletons `Eq` and `Acc`, that it refuses a
/// multi-constructor proposition and a one-constructor proposition storing data, and that the
/// eliminators which cross the line are DERIVED kernel-checked definitions rather than axioms.
#[test]
fn eq_and_acc_cross_the_prop_type_line_and_a_data_carrying_prop_does_not() {
    let mut ctx = std_ctx();

    // `Eq` and `Acc` map onto the prelude — re-declaring either re-points the machinery that
    // names them — and both are registered as INDEXED families with their Lean parameter split.
    assert_eq!(ctx.inductive_num_params("Eq"), 2, "Eq has numParams 2");
    assert_eq!(ctx.inductive_num_indices("Eq"), 1, "…and numIndices 1");
    assert_eq!(ctx.inductive_num_params("Acc"), 2, "Acc has numParams 2");
    assert_eq!(ctx.inductive_num_indices("Acc"), 1, "…and numIndices 1");

    // Their eliminators are kernel-CHECKED DEFINITIONS, not axioms: LOGOS derives J and the
    // well-founded eliminator instead of assuming them, so both leave the trusted base.
    for name in ["Eq_rec_dep", "Eq_rec", "Acc_rec"] {
        assert!(
            ctx.is_definition(name),
            "{name} must be a definition, never an add_declaration"
        );
        match double_check(&ctx, &g(name)) {
            DoubleCheck::Agreed => {}
            other => panic!("both kernels must certify the derived {name}, got {other:?}"),
        }
    }

    // `Acc_rec`'s motive lands in a LEVEL VARIABLE, as Lean's `Acc.rec` motive lands in
    // `Sort u_1`: the gate GRANTED large elimination at every level — at `Type 0`, which is what
    // makes well-founded recursion typecheck at all, and above it.
    let acc_rec_ty = infer_type(&ctx, &g("Acc_rec")).expect("Acc_rec is typed");
    let motive_target = |rec_ty: &Term| {
        let mut target = nth_pi_domain(rec_ty, 2).expect("A, R, then the motive");
        while let Term::Pi { body_type, .. } = target {
            target = *body_type;
        }
        target
    };
    let level = match motive_target(&acc_rec_ty) {
        Term::Sort(Universe::Var(level)) => level,
        other => panic!(
            "Acc is a subsingleton — its eliminator's motive must reach every sort, got {other}"
        ),
    };
    for at in [Universe::Type(0), Universe::Type(1)] {
        let instance = instantiate_universes(
            &acc_rec_ty,
            &[(level.clone(), at.clone())].into_iter().collect(),
        );
        assert_eq!(
            motive_target(&instance),
            Term::Sort(at.clone()),
            "Acc's eliminator reaches {at:?}: a proof of accessibility eliminates into data"
        );
    }

    // `Eq` large-eliminates directly: transport is a `Match` on a `Prop` returning a `Type`
    // value. One constructor, whose only arguments are the inductive's own parameters.
    let transport = match_(
        apps(g("refl"), &[nat(), g("Zero")]),
        lam(
            "y",
            nat(),
            lam("h", apps(g("Eq"), &[nat(), g("Zero"), v("y")]), nat()),
        ),
        vec![succ(g("Zero"))],
    );
    let t_ty = infer_type(&ctx, &transport).expect("eliminating Eq into Type must be allowed");
    assert!(
        same_type(&ctx, &t_ty, &nat()),
        "transport lands in Nat, got {t_ty}"
    );
    assert_eq!(
        normalize(&ctx, &transport),
        succ(g("Zero")),
        "…and it ι-computes"
    );
    match double_check(&ctx, &transport) {
        DoubleCheck::Agreed => {}
        other => panic!("both kernels must certify the Eq large elimination, got {other:?}"),
    }

    // THE REFUSING SIDE, one: MORE THAN ONE CONSTRUCTOR is never a subsingleton, whatever the
    // arguments are — three nullary proofs still may not pick a `Type`-level value.
    ctx.add_inductive("Tri", prop());
    for c in ["tria", "trib", "tric"] {
        ctx.add_constructor_checked(c, "Tri", g("Tri"))
            .expect("a nullary Prop constructor registers");
    }
    ctx.add_declaration("tri", g("Tri"));
    let tri_to_nat = match_(
        g("tri"),
        lam("_", g("Tri"), nat()),
        vec![g("Zero"), succ(g("Zero")), succ(succ(g("Zero")))],
    );
    assert!(
        matches!(
            infer_type(&ctx, &tri_to_nat),
            Err(KernelError::InvalidMotive(_))
        ),
        "a three-constructor proposition must not large-eliminate: {:?}",
        infer_type(&ctx, &tri_to_nat)
    );
    // …while the same elimination into `Prop` is never gated.
    let tri_to_prop = match_(
        g("tri"),
        lam("_", g("Tri"), g("True")),
        vec![g("I"), g("I"), g("I")],
    );
    assert!(
        infer_type(&ctx, &tri_to_prop).is_ok(),
        "Prop → Prop is always legal"
    );

    // THE REFUSING SIDE, two: the verdict is NOT about the constructor count alone. One
    // constructor that stores a `Nat` — data the indices do not determine — is not a
    // subsingleton either, and Lean refuses it for the same reason.
    ctx.add_inductive("Sized", prop());
    ctx.add_constructor_checked("smk", "Sized", arrow(nat(), arrow(g("True"), g("Sized"))))
        .expect("impredicative Prop admits the constructor itself");
    ctx.add_declaration("sz", g("Sized"));
    let sized_to_nat = match_(
        g("sz"),
        lam("_", g("Sized"), nat()),
        vec![lam("n", nat(), lam("t", g("True"), v("n")))],
    );
    assert!(
        matches!(
            infer_type(&ctx, &sized_to_nat),
            Err(KernelError::InvalidMotive(_))
        ),
        "a one-constructor proposition STORING DATA must not large-eliminate — otherwise a proof \
         yields the Nat it was built from while proof irrelevance says any two such proofs are \
         equal: {:?}",
        infer_type(&ctx, &sized_to_nat)
    );
    let sized_to_prop = match_(
        g("sz"),
        lam("_", g("Sized"), g("True")),
        vec![lam("n", nat(), lam("t", g("True"), v("t")))],
    );
    assert!(
        infer_type(&ctx, &sized_to_prop).is_ok(),
        "…but the Prop-valued elimination of the same proposition stays legal"
    );

    // The DOWNWARD direction is free: cumulativity lets a `Type 0` inductive be eliminated into
    // `Prop` with no gate at all.
    register_colour(&mut ctx);
    let colour_to_prop = match_(
        g("red"),
        lam("_", g("Colour"), prop()),
        vec![g("True"), g("False"), g("False")],
    );
    assert!(
        infer_type(&ctx, &colour_to_prop).is_ok(),
        "eliminating a Type 0 inductive into Prop needs no gate"
    );
}

// =============================================================================
// T03-018 · `noConfusion` / `injection`
// =============================================================================

#[test]
fn constructor_disjointness_and_injectivity_come_with_the_declaration() {
    let mut ctx = std_ctx();
    register_colour(&mut ctx);

    // CONTROL: for a `Type 0` inductive the principle is DERIVABLE by hand — a discriminating
    // family built with `Match`, then transport. This is what a translator must synthesise.
    ctx.add_definition(
        "isRed".to_string(),
        arrow(g("Colour"), prop()),
        lam(
            "c",
            g("Colour"),
            match_(
                v("c"),
                lam("_", g("Colour"), prop()),
                vec![g("True"), g("False"), g("False")],
            ),
        ),
    );
    assert_eq!(
        normalize(&ctx, &app(g("isRed"), g("red"))),
        g("True"),
        "the discriminating family must separate red…"
    );
    assert_eq!(
        normalize(&ctx, &app(g("isRed"), g("green"))),
        g("False"),
        "…from green — transporting True along red = green then yields False"
    );

    // CONTROL: the injectivity half is likewise hand-buildable, but only WITH a default for the
    // other constructor — `Match` has no wildcard and no impossible-case elimination.
    ctx.add_inductive("Chain", ty0());
    ctx.add_constructor_checked("cstop", "Chain", g("Chain"))
        .expect("cstop registers");
    ctx.add_constructor_checked(
        "clink",
        "Chain",
        arrow(nat(), arrow(g("Chain"), g("Chain"))),
    )
    .expect("clink is strictly positive");
    ctx.add_definition(
        "linkHead".to_string(),
        arrow(g("Chain"), nat()),
        lam(
            "c",
            g("Chain"),
            match_(
                v("c"),
                lam("_", g("Chain"), nat()),
                vec![g("Zero"), lam("a", nat(), lam("t", g("Chain"), v("a")))],
            ),
        ),
    );
    assert_eq!(
        normalize(
            &ctx,
            &app(
                g("linkHead"),
                apps(g("clink"), &[succ(g("Zero")), g("cstop")])
            )
        ),
        succ(g("Zero")),
        "the hand-built projection recovers the constructor argument"
    );

    // …and on a `Prop` the workaround is not merely unavailable, it is a category error:
    // proof irrelevance makes any two inhabitants of a proposition definitionally equal, so
    // constructor disjointness is FALSE there. Disjointness is a Type-level phenomenon.
    ctx.add_inductive("Tri", prop());
    for c in ["tria", "trib", "tric"] {
        ctx.add_constructor_checked(c, "Tri", g("Tri"))
            .expect("a Prop constructor registers");
    }
    assert!(
        defeq_for_test(&ctx, &g("tria"), &g("trib")),
        "proof irrelevance identifies two inhabitants of a Prop — `noConfusion` on a Prop is a \
         category error, not a missing feature"
    );

    // THE GAP: Lean generates `Chain.noConfusion`, `Chain.noConfusionType`, `Chain.link.injEq`
    // and `Chain.link.inj` at declaration time. LOGOS's registration entry points add a header
    // and constructors and nothing else; `derive_recursor` is the ONLY auxiliary it can
    // synthesise, and even that must be requested and registered by the caller.
    assert!(
        ctx.get_global("Chain_noConfusion").is_some(),
        "declaring an inductive must yield its constructor-disjointness principle. Every \
         `injection`, `cases h` on an equation, and `simp [injEq]` proof in a Lean development \
         names a constant that has no LOGOS counterpart, and axiomatising it would turn each of \
         those proofs into an assumption"
    );
}

// =============================================================================
// T03-019 · Impossible case in an indexed match
// =============================================================================

#[test]
fn an_index_contradictory_constructor_needs_no_branch() {
    let mut ctx = std_ctx();
    register_vect(&mut ctx);

    // `isEmpty : Vect Nat 0 → Bool`. The scrutinee's index is `Zero`, so `vcons` (index
    // `Succ n`) provably cannot occur — Lean discharges that branch by index unification and
    // emits nothing for it.
    let empty = app(g("vnil"), nat());
    let motive = lam(
        "n",
        nat(),
        lam("z", apps(g("Vect"), &[nat(), v("n")]), g("Bool")),
    );

    // CONTROL: `Term::Hole` is the most dangerous filler — it must NOT be accepted as a branch.
    let with_hole = match_(empty.clone(), motive.clone(), vec![g("true"), Term::Hole]);
    assert!(
        infer_type(&ctx, &with_hole).is_err(),
        "a Hole must never stand in for an impossible branch — types_equal treats it as matching \
         anything, which would carry an unchecked subterm into the context"
    );

    // THE GAP: coverage is a bare count. One case per constructor, always, even when the
    // constructor's declared indices contradict the scrutinee's.
    let discharged = match_(empty, motive, vec![g("true")]);
    let outcome = infer_type(&ctx, &discharged);
    assert!(
        outcome.is_ok(),
        "a constructor whose declared result index cannot unify with the discriminant's must \
         need no branch. LOGOS's coverage rule is `cases.len() == constructors.len()` with no \
         index unifier and no absurd-branch marker, so the translator must invent an inhabitant \
         of an unreachable branch's type — and for `vtail`, whose impossible branch owes a \
         `Vect A n` with no value in scope, there is none short of the constructor disjointness \
         we also do not generate: {outcome:?}"
    );
}

// =============================================================================
// T03-020 · Zero-constructor inductive
// =============================================================================

#[test]
fn an_empty_inductive_eliminates_with_no_cases_at_all() {
    let mut ctx = std_ctx();

    // The prelude's `False` is exactly this shape: an inductive header and NO constructor call.
    assert!(
        ctx.get_constructors("False").is_empty(),
        "False must stay uninhabited"
    );
    ctx.add_declaration("h", g("False"));

    // Ex falso into every sort. Zero cases is the CORRECT shape, not a degenerate one, and the
    // subsingleton rule (zero constructors ⇒ true) is what licenses the large elimination.
    for (name, target) in [("Nat", nat()), ("Type", ty0())] {
        let elim = match_(g("h"), lam("_", g("False"), target.clone()), vec![]);
        let ty = infer_type(&ctx, &elim)
            .unwrap_or_else(|e| panic!("False.elim into {name} must typecheck: {e:?}"));
        assert!(
            same_type(&ctx, &ty, &target),
            "False.elim lands in {name}, got {ty}"
        );
        match double_check(&ctx, &elim) {
            DoubleCheck::Agreed => {}
            other => panic!("both kernels must certify False.elim into {name}, got {other:?}"),
        }
    }

    // A filler branch is a hard error — the count rule is exact, so a translator that habitually
    // synthesises one must special-case the empty inductive.
    assert!(
        matches!(
            infer_type(
                &ctx,
                &match_(g("h"), lam("_", g("False"), nat()), vec![g("Zero")])
            ),
            Err(KernelError::WrongNumberOfCases {
                expected: 0,
                found: 1
            })
        ),
        "one case against zero constructors is WrongNumberOfCases"
    );

    // A USER zero-constructor `Type 0` — Lean's `Empty`, which the prelude does not provide, so
    // it must be emitted at the right sort and under a name that does not collide.
    ctx.add_inductive("Nothing", ty0());
    ctx.add_declaration("n", g("Nothing"));
    assert!(
        ctx.get_constructors("Nothing").is_empty(),
        "no dummy constructor — that inhabits it"
    );
    let nelim = match_(g("n"), lam("_", g("Nothing"), nat()), vec![]);
    assert!(
        infer_type(&ctx, &nelim).is_ok(),
        "an empty Type 0 inductive eliminates the same way"
    );
    let (_, nrec) = derive_recursor(&ctx, "Nothing").expect("the empty eliminator derives");
    match double_check(&ctx, &nrec) {
        DoubleCheck::Agreed => {}
        other => panic!("both kernels must certify the empty recursor, got {other:?}"),
    }

    // An empty inductive WITH an index: arity 1, zero constructors, and the params/indices split
    // is easy to overlook precisely because there are no constructors to disagree.
    ctx.add_indexed_inductive("NeverAt", arrow(nat(), prop()), 0);
    assert_eq!(
        ctx.inductive_num_indices("NeverAt"),
        1,
        "the Nat is an INDEX, not a parameter"
    );
    ctx.add_declaration("na", app(g("NeverAt"), g("Zero")));
    let ielim = match_(
        g("na"),
        lam("i", nat(), lam("z", app(g("NeverAt"), v("i")), nat())),
        vec![],
    );
    assert!(
        infer_type(&ctx, &ielim).is_ok(),
        "the INDEXED empty inductive eliminates with zero cases too: {:?}",
        infer_type(&ctx, &ielim)
    );
}

// =============================================================================
// T03-021 · Recursor primitivity inversion — DIVERGE
// =============================================================================

/// **DIVERGE.** In Lean `Foo.rec` is a kernel primitive with its own iota rule and `match` is
/// compiled down to it. LOGOS is the exact inverse: `Term::Match` is a primitive of the term
/// language and the recursor is DERIVED from it — `fix rec. λindices. λx. match x …` — which
/// means the eliminator is an ordinary certified term that the independent second kernel
/// re-checks, rather than a primitive the trusted base must be trusted on. Every inductive's
/// induction principle is therefore evidence rather than axiom, and the two derived eliminators
/// the prelude ships (`Eq_rec_dep`, `Acc_rec`) are kernel-checked definitions where Lean takes
/// J and well-founded recursion as primitive. As in Lean, declaring the type publishes its
/// recursor — here DERIVED and kernel-checked, never assumed. The cost that remains is named here:
/// the published recursor's motive target is pinned rather than universe-polymorphic (T01-007;
/// `derive_recursor_universe_poly` is the polymorphic door).
#[test]
fn the_recursor_is_derived_from_the_primitive_match_and_carries_the_hypotheses() {
    let mut ctx = std_ctx();
    register_tree(&mut ctx);

    // Declaring the inductive publishes `Tree_rec`, as Lean's `inductive` publishes `Tree.rec` —
    // and it is a DERIVED definition with a body both kernels certify, never an axiom.
    let published_ty = ctx
        .get_global("Tree_rec")
        .cloned()
        .expect("declaring an inductive publishes its recursor, as Lean's declaration does");
    let published = ctx
        .get_definition_body("Tree_rec")
        .cloned()
        .expect("the published recursor has a body — it is derived, not assumed");
    assert!(
        !ctx.iter_declarations().any(|(name, _)| name == "Tree_rec"),
        "the published recursor is not an axiom"
    );
    match double_check(&ctx, &published) {
        DoubleCheck::Agreed => {}
        other => panic!("both kernels must certify the published recursor, got {other:?}"),
    }

    let (rec_ty, rec_term) = derive_recursor(&ctx, "Tree").expect("Tree's recursor derives");
    assert!(
        same_type(&ctx, &published_ty, &rec_ty),
        "the published recursor is the derived one: {published_ty} vs {rec_ty}"
    );

    // The reported type is, by construction, the one the kernel certifies for the term.
    let inferred = infer_type(&ctx, &rec_term).expect("the derived recursor typechecks");
    assert!(
        same_type(&ctx, &inferred, &rec_ty),
        "derive_recursor's type is the certified one"
    );
    match double_check(&ctx, &rec_term) {
        DoubleCheck::Agreed => {}
        other => panic!("both kernels must certify the derived recursor, got {other:?}"),
    }

    // Built OUT OF the primitives: peel the λs and a `Fix` wrapping a `Match` is underneath.
    assert!(
        matches!(peel_lambdas(&rec_term), Term::Fix { .. }),
        "the recursor's body is a Fix over a Match — Match is the primitive, rec is the derived term"
    );

    // It supplies INDUCTION HYPOTHESES, which a bare `Match` does not. `node`'s minor premise
    // takes both recursive occurrences AND both of their results: four binders, not two.
    // Conflating `rec` with `casesOn` typechecks on a non-recursive type and breaks here.
    let node_premise = nth_pi_domain(&rec_ty, 2).expect("motive, leaf premise, node premise");
    assert_eq!(
        count_leading_pis(&node_premise),
        4,
        "node's minor premise is Π(l).Π(r). P l → P r → P (node l r): {node_premise}"
    );
    let leaf_premise = nth_pi_domain(&rec_ty, 1).expect("the leaf premise follows the motive");
    assert_eq!(
        count_leading_pis(&leaf_premise),
        1,
        "leaf's minor premise takes only its stored Nat: {leaf_premise}"
    );

    // …and the PUBLISHED recursor ι-COMPUTES, using the induction hypothesis the `Match` could
    // not have produced.
    let depth = apps(
        g("Tree_rec"),
        &[
            lam("_", g("Tree"), nat()),
            lam("n", nat(), v("n")),
            lam(
                "l",
                g("Tree"),
                lam(
                    "r",
                    g("Tree"),
                    lam("il", nat(), lam("ir", nat(), succ(v("ir")))),
                ),
            ),
            apps(
                g("node"),
                &[app(g("leaf"), g("Zero")), app(g("leaf"), succ(g("Zero")))],
            ),
        ],
    );
    assert_eq!(
        normalize(&ctx, &depth),
        succ(succ(g("Zero"))),
        "the derived recursor must reduce through the recursive call it supplies"
    );
}

// =============================================================================
// T03-022 · Constructor universe constraint — the Girard/Hurkens door
// =============================================================================

#[test]
fn a_constructor_may_not_store_a_field_above_its_inductives_sort() {
    let mut ctx = std_ctx();

    // `Bad : Type 0` with `mk : Type 0 → Bad` — a `Type 0` type quantifying over `Type 0`.
    ctx.add_inductive("Bad", ty0());
    let bad_mk = arrow(ty0(), g("Bad"));
    assert!(
        matches!(
            ctx.add_constructor_checked("bad_mk", "Bad", bad_mk.clone()),
            // A decided semantic refusal, not incomplete checking (constructor_field_universe_verdicts).
            Err(KernelError::InvalidDeclaration(ref m)) if m.contains("universe inconsistency")
        ),
        "a Type 0 inductive may not store a Type 0-typed field (whose sort is Type 1)"
    );
    assert!(
        ctx.get_global("bad_mk").is_none() && ctx.get_constructors("Bad").is_empty(),
        "a refused constructor registers nothing"
    );
    // The same violation through a bound type variable — the shape that looks like ordinary
    // parametricity and is the one that actually appears.
    assert!(
        ctx.add_constructor_checked("bad_pk", "Bad", pi("A", ty0(), arrow(v("A"), g("Bad"))))
            .is_err(),
        "`Π(A : Type 0). A → Bad` stores the TYPE A, not just a value of it"
    );

    // THE TCB BOUNDARY, pinned: the low-level `add_constructor` is deliberately unchecked — it is
    // what the prelude and `add_structure` use — so `add_constructor_checked` /
    // `add_mutual_inductives` are the ONLY registration paths a caller may use for user input.
    ctx.add_constructor("bad_unchecked", "Bad", bad_mk);
    assert!(
        ctx.get_global("bad_unchecked").is_some(),
        "add_constructor performs no check at all; that is the trusted-base boundary and every \
         user-facing registration must go through the checked path instead"
    );

    // THE ACCEPTING SIDE — the discrimination must be exact in both directions.
    let mut ok = std_ctx();
    // One universe higher, and the same constructor is legal.
    ok.add_inductive("Good", ty1());
    ok.add_constructor_checked("good_mk", "Good", arrow(ty0(), g("Good")))
        .expect("a Type 1 inductive may store a Type 0-typed field");
    // Impredicative `Prop` admits a field of ANY sort.
    ok.add_inductive("PropPackage", prop());
    ok.add_constructor_checked(
        "pp_mk",
        "PropPackage",
        pi("A", ty0(), arrow(v("A"), g("PropPackage"))),
    )
    .expect("Prop is exempt — impredicativity, exactly as in Coq/Lean");
    // A PARAMETER is exempt: `Box A` storing an `A` is fine; the parameter is not a stored field.
    ok.add_indexed_inductive("Box", pi("A", ty0(), ty0()), 1);
    ok.add_constructor_checked(
        "box_mk",
        "Box",
        pi("A", ty0(), arrow(v("A"), app(g("Box"), v("A")))),
    )
    .expect("the leading num_params binders are exempt");

    // The MUTUAL path is checked too, in a temp environment carrying the block's headers, before
    // anything is mutated.
    let bad_block = vec![
        MutualInductive {
            name: "MA".to_string(),
            sort: ty0(),
            num_params: 0,
            constructors: vec![("ma".to_string(), arrow(ty0(), arrow(g("MB"), g("MA"))))],
        },
        MutualInductive {
            name: "MB".to_string(),
            sort: ty0(),
            num_params: 0,
            constructors: vec![("mstop".to_string(), g("MB"))],
        },
    ];
    let mut mctx = std_ctx();
    assert!(
        mctx.add_mutual_inductives(&bad_block).is_err(),
        "a universe violation inside a mutual block must be refused"
    );
    assert!(
        !mctx.is_inductive("MA") && !mctx.is_inductive("MB") && mctx.get_global("ma").is_none(),
        "…and transactionally: nothing is added"
    );
    // The same block one universe up is accepted.
    let mut mok = std_ctx();
    assert!(
        mok.add_mutual_inductives(&[
            MutualInductive {
                name: "MA1".to_string(),
                sort: ty1(),
                num_params: 0,
                constructors: vec![("ma1".to_string(), arrow(ty0(), arrow(g("MB1"), g("MA1"))))],
            },
            MutualInductive {
                name: "MB1".to_string(),
                sort: ty1(),
                num_params: 0,
                constructors: vec![
                    ("mb1".to_string(), arrow(ty0(), g("MB1"))),
                    ("mstop1".to_string(), g("MB1")),
                ],
            },
        ])
        .is_ok(),
        "both members at Type 1 storing Type-valued fields is legal and checked"
    );
}

// =============================================================================
// T03-023 · `deriving DecidableEq`
// =============================================================================

#[test]
fn a_declared_inductive_gets_a_decidable_equality_instance() {
    let mut ctx = std_ctx();
    register_colour(&mut ctx);

    let eq_at = |t: Term, a: Term, b: Term| apps(g("Eq"), &[t, a, b]);
    let decide_eq =
        |t: Term, a: Term, b: Term, inst: Term| apps(g("decide"), &[eq_at(t, a, b), inst]);

    // CONTROL: LOGOS has exactly TWO decidable equalities, both hand-written, and they work.
    assert_eq!(
        normalize(
            &ctx,
            &decide_eq(
                nat(),
                g("Zero"),
                g("Zero"),
                apps(g("decEqNat"), &[g("Zero"), g("Zero")])
            )
        ),
        g("true"),
        "decEqNat decides Nat equality"
    );
    assert_eq!(
        normalize(
            &ctx,
            &decide_eq(
                nat(),
                g("Zero"),
                succ(g("Zero")),
                apps(g("decEqNat"), &[g("Zero"), succ(g("Zero"))])
            )
        ),
        g("false"),
        "…in both directions"
    );

    // CONTROL: `decEqNat` decides equality of `Nat` and of NOTHING else. Reusing it for a type
    // whose payload happens to be a `Nat` yields an instance that compares one field and ignores
    // the constructor tag, and `decide`'s `Bool` result hides it.
    assert!(
        infer_type(&ctx, &apps(g("decEqNat"), &[g("red"), g("green")])).is_err(),
        "decEqNat must not accept values of another type"
    );

    // THE GAP: Lean's `deriving DecidableEq` generates the decision procedure AND registers it as
    // an instance. LOGOS has no derive-handler registry at all, so every one must be transcribed
    // by hand — an n-constructor type needing n² branches, because `Match` has no wildcard.
    let mut mctx = MetaCtx::new();
    let required = app(g("Decidable"), eq_at(g("Colour"), g("red"), g("green")));
    let resolved = resolve_instance(&ctx, &mut mctx, &required);
    let Some(inst) = resolved else {
        panic!(
            "declaring an inductive must yield a `Decidable (Eq Colour a b)` instance findable by \
             resolution. Nothing in LOGOS derives one: the prelude ships decEqBool and decEqNat, \
             hand-written, and there is no derive mechanism to extend"
        );
    };
    assert_eq!(
        normalize(&ctx, &decide_eq(g("Colour"), g("red"), g("green"), inst)),
        g("false"),
        "a derived decision procedure must separate DIFFERENT constructors"
    );
}

// =============================================================================
// T03-024 · `Quot` / `Quotient`
// =============================================================================

#[test]
fn quotients_offer_a_dependent_eliminator_over_their_classes() {
    let mut ctx = std_ctx();

    // The relation and the carrier LOGOS's `Quot` accepts: a `Type 0` carrier.
    let rel = lam("a", nat(), lam("b", nat(), g("True")));
    let quot = apps(g("Quot"), &[nat(), rel.clone()]);
    let mk = |a: Term| apps(g("Quot_mk"), &[nat(), rel.clone(), a]);
    let f = lam("n", nat(), g("Zero"));
    let resp = lam(
        "a",
        nat(),
        lam(
            "b",
            nat(),
            lam("_", g("True"), apps(g("refl"), &[nat(), g("Zero")])),
        ),
    );

    // CONTROL: the ONE computation rule quotients have — `lift f h (mk a) ⤳ f a`.
    let lifted = apps(
        g("Quot_lift"),
        &[nat(), rel.clone(), nat(), f.clone(), resp, mk(g("Zero"))],
    );
    let lifted_ty =
        infer_type(&ctx, &lifted).expect("Quot_lift with its respect obligation typechecks");
    assert!(
        same_type(&ctx, &lifted_ty, &nat()),
        "Quot_lift lands in B, got {lifted_ty}"
    );
    assert_eq!(
        normalize(&ctx, &lifted),
        g("Zero"),
        "Quot_lift f h (Quot_mk A r a) must reduce to f a"
    );

    // CONTROL: `Quot` is an AXIOM, not an inductive — deliberately. An inductive `Quot` could be
    // MATCHED on, and a match recovering the representative contradicts `Quot_sound`.
    assert!(
        !ctx.is_inductive("Quot"),
        "Quot must not be an inductive — matching on a quotient would break Quot_sound"
    );
    ctx.add_declaration("q", quot.clone());
    assert!(
        matches!(
            infer_type(
                &ctx,
                &match_(g("q"), lam("_", quot.clone(), nat()), vec![g("Zero")])
            ),
            Err(KernelError::NotAnInductive(_))
        ),
        "a Match on a quotient value must be refused outright"
    );

    // CONTROL: `Quot_sound` — related representatives give equal classes.
    let sound = apps(
        g("Quot_sound"),
        &[nat(), rel.clone(), g("Zero"), succ(g("Zero")), g("I")],
    );
    let sound_ty = infer_type(&ctx, &sound).expect("Quot_sound is applicable");
    assert!(
        same_type(
            &ctx,
            &sound_ty,
            &apps(g("Eq"), &[quot.clone(), mk(g("Zero")), mk(succ(g("Zero")))])
        ),
        "Quot_sound : r a b → Eq (Quot A r) (mk a) (mk b), got {sound_ty}"
    );

    // …and Lean's `Setoid`/`Quotient` layer exists over it, as one-liners over `Quot` that cost
    // the trusted base nothing: `Setoid` is a structure, `Quotient` a transparent definition, and
    // `Quotient.lift f h (Quotient.mk a)` computes to `f a` through the primitive.
    assert!(ctx.is_inductive("Setoid"), "`Setoid` is a structure");
    for name in ["Quotient", "Quotient_mk", "Quotient_lift", "Quotient_sound"] {
        assert!(
            ctx.get_definition_body(name).is_some()
                && !ctx.iter_declarations().any(|(n, _)| n == name),
            "`{name}` is a checked definition over `Quot`, never an axiom"
        );
    }
    ctx.add_declaration("sNat", app(g("Setoid"), nat()));
    let related = |x: Term, y: Term| apps(g("Setoid_r"), &[nat(), g("sNat"), x, y]);
    ctx.add_declaration(
        "respectZero",
        pi(
            "x",
            nat(),
            pi(
                "y",
                nat(),
                arrow(
                    related(v("x"), v("y")),
                    apps(g("Eq"), &[nat(), g("Zero"), g("Zero")]),
                ),
            ),
        ),
    );
    let lifted_class = apps(
        g("Quotient_lift"),
        &[
            nat(),
            g("sNat"),
            nat(),
            lam("n", nat(), g("Zero")),
            g("respectZero"),
            apps(g("Quotient_mk"), &[nat(), g("sNat"), succ(g("Zero"))]),
        ],
    );
    assert!(
        same_type(
            &ctx,
            &infer_type(&ctx, &lifted_class).expect("the wrapper typechecks"),
            &nat()
        ),
        "`Quotient.lift` lands in its codomain"
    );
    assert_eq!(
        normalize(&ctx, &lifted_class),
        g("Zero"),
        "`Quotient.lift f h (Quotient.mk a)` reduces to `f a` through `Quot`'s own rule"
    );

    // THE GAP: LOGOS offers exactly two eliminators — `Quot_lift`, non-dependent and into
    // `Type 0`, and `Quot_ind`, whose motive is restricted to `Prop`. Lean's `Quot.rec` takes a
    // motive that VARIES over the quotient and a respect obligation stated up to transport;
    // neither of ours can express it, and approximating it with `Quot_lift` at the motive's
    // instantiation discards the transport and produces a term of a different type.
    assert!(
        ctx.get_global("Quot_ind").is_some(),
        "the Prop-motive eliminator must exist as the baseline"
    );
    assert!(
        ctx.get_global("Quot_rec").is_some(),
        "a quotient needs a DEPENDENT eliminator. `Quot_lift`'s motive is a fixed `B : Type 0` \
         and `Quot_ind`'s is `Quot A r → Prop`, so a definition by `Quot.rec` into a family over \
         the quotient has no LOGOS counterpart at all"
    );
}
