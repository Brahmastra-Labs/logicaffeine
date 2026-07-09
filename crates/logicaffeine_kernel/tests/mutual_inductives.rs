//! Mutual (and later nested) inductive families. A block of inductives may
//! reference one another in their constructors — `Even`/`Odd`, `Tree`/`Forest`.
//! Strict positivity must be checked over the WHOLE block (a sibling occurrence
//! is a recursive occurrence; a sibling in a negative position is still a
//! paradox and must be rejected), and each type gets its induction principle.
//!
//! K3a here: block registration + cross-block positivity. The mutual recursor and
//! the mutual termination guard follow in K3b.

use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{
    derive_recursor, double_check, infer_type, is_subtype, normalize, recheck, Context,
    DoubleCheck, MutualInductive, Term, Universe,
};

/// Alpha-aware type equality (binder names may differ, e.g. `Π(n:Nat).Bool` ≡ `Nat→Bool`).
fn same_type(ctx: &Context, a: &Term, b: &Term) -> bool {
    is_subtype(ctx, a, b) && is_subtype(ctx, b, a)
}

fn g(n: &str) -> Term {
    Term::Global(n.to_string())
}
fn v(n: &str) -> Term {
    Term::Var(n.to_string())
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn pi(p: &str, t: Term, b: Term) -> Term {
    Term::Pi { param: p.to_string(), param_type: Box::new(t), body_type: Box::new(b) }
}
fn arrow(a: Term, b: Term) -> Term {
    pi("_", a, b)
}
fn lam(p: &str, t: Term, b: Term) -> Term {
    Term::Lambda { param: p.to_string(), param_type: Box::new(t), body: Box::new(b) }
}
fn match_(d: Term, motive: Term, cases: Vec<Term>) -> Term {
    Term::Match { discriminant: Box::new(d), motive: Box::new(motive), cases }
}
fn mutfix(defs: &[(&str, Term)], index: usize) -> Term {
    Term::MutualFix {
        defs: defs.iter().map(|(n, b)| (n.to_string(), b.clone())).collect(),
        index,
    }
}
fn apps(f: Term, xs: &[Term]) -> Term {
    xs.iter().fold(f, |a, x| app(a, x.clone()))
}
fn nat() -> Term {
    g("Nat")
}
fn bool_t() -> Term {
    g("Bool")
}
fn prop() -> Term {
    Term::Sort(Universe::Prop)
}
fn ty0() -> Term {
    Term::Sort(Universe::Type(0))
}
fn succ(n: Term) -> Term {
    app(g("Succ"), n)
}

/// The canonical NON-parametric mutual block over `Type`: `Tree`/`Forest`.
/// `Tree := node : Nat → Forest → Tree`,
/// `Forest := fnil : Forest | fcons : Tree → Forest → Forest`.
fn tree_forest_block() -> Vec<MutualInductive> {
    vec![
        MutualInductive {
            name: "Tree".to_string(),
            sort: ty0(),
            num_params: 0,
            constructors: vec![("node".to_string(), arrow(nat(), arrow(g("Forest"), g("Tree"))))],
        },
        MutualInductive {
            name: "Forest".to_string(),
            sort: ty0(),
            num_params: 0,
            constructors: vec![
                ("fnil".to_string(), g("Forest")),
                ("fcons".to_string(), arrow(g("Tree"), arrow(g("Forest"), g("Forest")))),
            ],
        },
    ]
}
fn std_ctx() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx
}

/// The mutual `isEven`/`isOdd` over `Nat`, returning `Bool`. Each recurses on the
/// structural predecessor and calls its SIBLING. `bad = true` breaks termination by
/// recursing on the un-decremented parameter. Returns the `index`-th component.
fn is_even_odd(index: usize, bad: bool) -> Term {
    let bool_motive = lam("_", nat(), bool_t());
    // isOdd's Succ argument: `isEven m` normally, `isEven n` (non-decreasing) if bad.
    let iseven_body = lam(
        "n",
        nat(),
        match_(
            v("n"),
            bool_motive.clone(),
            vec![g("true"), lam("m", nat(), app(v("isOdd"), if bad { v("n") } else { v("m") }))],
        ),
    );
    let isodd_body = lam(
        "n",
        nat(),
        match_(
            v("n"),
            bool_motive,
            vec![g("false"), lam("m", nat(), app(v("isEven"), v("m")))],
        ),
    );
    mutfix(&[("isEven", iseven_body), ("isOdd", isodd_body)], index)
}

/// The canonical mutual block: `Even`/`Odd` over `Nat`.
/// `Even : Nat → Prop`, `Odd : Nat → Prop`,
/// `even_zero : Even Zero`, `even_succ : Π(n). Odd n → Even (Succ n)`,
/// `odd_succ : Π(n). Even n → Odd (Succ n)`. Each constructor mentions its SIBLING
/// as a strictly-positive recursive argument.
fn even_odd_block() -> Vec<MutualInductive> {
    vec![
        MutualInductive {
            name: "Even".to_string(),
            sort: pi("n", nat(), prop()),
            num_params: 0,
            constructors: vec![
                ("even_zero".to_string(), app(g("Even"), g("Zero"))),
                (
                    "even_succ".to_string(),
                    pi("n", nat(), arrow(app(g("Odd"), v("n")), app(g("Even"), succ(v("n"))))),
                ),
            ],
        },
        MutualInductive {
            name: "Odd".to_string(),
            sort: pi("n", nat(), prop()),
            num_params: 0,
            constructors: vec![(
                "odd_succ".to_string(),
                pi("n", nat(), arrow(app(g("Even"), v("n")), app(g("Odd"), succ(v("n"))))),
            )],
        },
    ]
}

#[test]
fn even_odd_block_registers_and_typechecks() {
    let mut ctx = std_ctx();
    ctx.add_mutual_inductives(&even_odd_block()).expect("Even/Odd block registers");
    for name in ["Even", "Odd", "even_zero", "even_succ", "odd_succ"] {
        assert!(infer_type(&ctx, &g(name)).is_ok(), "{name} must be registered and type-check");
    }
    // `Even (Succ (Succ Zero))` is a well-formed proposition (Even 2).
    let even_two = app(g("Even"), succ(succ(g("Zero"))));
    let sort = infer_type(&ctx, &even_two).expect("Even 2 type-checks");
    assert_eq!(sort, prop(), "Even 2 must be a Prop");
}

#[test]
fn cross_block_negative_occurrence_is_rejected() {
    // THE MUTUAL FENCE: a constructor that puts a SIBLING in a negative position —
    // `bad : Π(n). (Even n → False) → Odd n` — is a cross-block paradox and must be
    // rejected, exactly as a self-negative occurrence is. Positivity is checked
    // over the whole block, so `Even` in the domain of an inner arrow is caught even
    // though the constructor belongs to `Odd`.
    let mut block = even_odd_block();
    block[1].constructors.push((
        "bad".to_string(),
        pi(
            "n",
            nat(),
            arrow(arrow(app(g("Even"), v("n")), g("False")), app(g("Odd"), succ(v("n")))),
        ),
    ));
    let mut ctx = std_ctx();
    assert!(
        ctx.add_mutual_inductives(&block).is_err(),
        "a cross-block negative occurrence (Even n → False) → Odd n must be rejected"
    );
    // And the rejection must be TRANSACTIONAL — no partial registration leaks.
    assert!(infer_type(&ctx, &g("Even")).is_err(), "a rejected block must register nothing");
    assert!(infer_type(&ctx, &g("odd_succ")).is_err(), "a rejected block must register nothing");
}

// --- K3b: the mutual fixpoint machinery (typing + guard + reduction) ---------

#[test]
fn mutual_fixpoint_typechecks_computes_and_is_guarded() {
    // `isEven`/`isOdd` — genuine mutual recursion, each recursing on the structural
    // predecessor and calling its SIBLING. The kernel must (a) type it `Nat → Bool`,
    // (b) accept it via the MUTUAL termination guard, and (c) COMPUTE by unfolding both
    // components: isEven 2 = true, isEven 1 = false, isEven 3 = false.
    let ctx = std_ctx();
    let iseven = is_even_odd(0, false);
    // (a) + (b): types, which runs the guard.
    let ty = infer_type(&ctx, &iseven).expect("isEven type-checks (mutual guard accepts)");
    assert!(same_type(&ctx, &ty, &arrow(nat(), bool_t())), "isEven : Nat → Bool, got {ty}");
    // (c): computes across the mutual boundary.
    let two = succ(succ(g("Zero")));
    let three = succ(two.clone());
    assert_eq!(normalize(&ctx, &app(iseven.clone(), two)), g("true"), "isEven 2 = true");
    assert_eq!(normalize(&ctx, &app(iseven.clone(), succ(g("Zero")))), g("false"), "isEven 1 = false");
    assert_eq!(normalize(&ctx, &app(iseven, three)), g("false"), "isEven 3 = false");

    // The sibling component (index 1) is `isOdd : Nat → Bool` and computes dually.
    let isodd = is_even_odd(1, false);
    assert!(
        same_type(&ctx, &infer_type(&ctx, &isodd).unwrap(), &arrow(nat(), bool_t())),
        "isOdd : Nat → Bool"
    );
    assert_eq!(normalize(&ctx, &app(isodd, succ(succ(g("Zero"))))), g("false"), "isOdd 2 = false");
}

#[test]
fn tree_forest_mutual_recursor_derives_and_typechecks() {
    // The headline: declaring a mutual block auto-derives an induction/recursion
    // principle for EACH member, sharing one `MutualFix`. Both recursors must derive
    // and type-check — and type-checking runs the mutual termination guard over the
    // shared fixpoint, so a green result certifies the whole mutual machinery.
    let mut ctx = std_ctx();
    ctx.add_mutual_inductives(&tree_forest_block()).expect("Tree/Forest registers");

    let (tree_ty, tree_rec) = derive_recursor(&ctx, "Tree").expect("Tree_rec derives");
    let (_forest_ty, forest_rec) = derive_recursor(&ctx, "Forest").expect("Forest_rec derives");

    // The derived terms are certified by the kernel (mutual guard included).
    assert!(infer_type(&ctx, &tree_rec).is_ok(), "Tree_rec body type-checks: {tree_ty}");
    assert!(infer_type(&ctx, &forest_rec).is_ok(), "Forest_rec body type-checks");

    // Tree_rec takes TWO motives (one per block member) — it is a genuinely mutual
    // eliminator, not a single-type recursor. Its type opens `Π(P0). Π(P1). …`.
    if let Term::Pi { param, .. } = &tree_ty {
        assert_eq!(param, "P0", "Tree_rec's first binder is the Tree motive P0");
    } else {
        panic!("Tree_rec type must be a Π, got {tree_ty}");
    }
}

#[test]
fn tree_forest_mutual_recursor_computes() {
    // THE HEADLINE COMPUTATION: define `forestLength : Forest → Nat` (the number of
    // top-level trees) via the auto-derived `Forest_rec`, and evaluate it on a forest
    // of two trees. The mutual recursor must COMPUTE — reducing across the shared
    // fixpoint (Forest's fcons case recurses into the tail, and the derived recursor
    // also eliminates the head Tree) — to `Succ (Succ Zero)`.
    let mut ctx = std_ctx();
    ctx.add_mutual_inductives(&tree_forest_block()).expect("Tree/Forest registers");
    let (_forest_ty, forest_rec) = derive_recursor(&ctx, "Forest").expect("Forest_rec derives");

    // Motives: both members measured into Nat.
    let p_tree = lam("_", g("Tree"), nat());
    let p_forest = lam("_", g("Forest"), nat());
    // node minor: Π(n:Nat). Π(f:Forest). Π(ihf:Nat). Nat  — a tree contributes nothing
    // to the top-level length, so return Zero.
    let f_node = lam("n", nat(), lam("f", g("Forest"), lam("ihf", nat(), g("Zero"))));
    // fnil minor: Nat — the empty forest has length Zero.
    let f_fnil = g("Zero");
    // fcons minor: Π(t:Tree). Π(f:Forest). Π(iht:Nat). Π(ihf:Nat). Nat — one more than
    // the tail's length (`Succ ihf`), threading the tail IH across the mutual boundary.
    let f_fcons = lam(
        "t",
        g("Tree"),
        lam("f", g("Forest"), lam("iht", nat(), lam("ihf", nat(), succ(v("ihf"))))),
    );

    // A forest of two (leaf) trees: fcons t (fcons t fnil), t = node Zero fnil.
    let leaf = apps(g("node"), &[g("Zero"), g("fnil")]);
    let forest = apps(g("fcons"), &[leaf.clone(), apps(g("fcons"), &[leaf, g("fnil")])]);

    let length =
        apps(forest_rec, &[p_tree, p_forest, f_node, f_fnil, f_fcons, forest]);
    // Well-typed (the minors match the derived recursor's premises) …
    let ty = infer_type(&ctx, &length).expect("forestLength application type-checks");
    assert!(same_type(&ctx, &ty, &nat()), "forestLength forest : Nat, got {ty}");
    // … and computes to 2.
    assert_eq!(
        normalize(&ctx, &length),
        succ(succ(g("Zero"))),
        "a forest of two trees has length 2"
    );
}

/// A PARAMETRIC mutual block `Tree A` / `Forest A` (element type `A`). Its recursor must
/// derive with the shared parameter bound, type-check, and be two-kernel certified —
/// closing the "non-parametric mutual only" gap.
fn tree_forest_a_block() -> Vec<MutualInductive> {
    vec![
        MutualInductive {
            name: "TreeA".to_string(),
            sort: pi("A", ty0(), ty0()), // TreeA : Type → Type
            num_params: 1,
            constructors: vec![(
                "nodeA".to_string(),
                // Π(A). A → ForestA A → TreeA A
                pi("A", ty0(), arrow(v("A"), arrow(app(g("ForestA"), v("A")), app(g("TreeA"), v("A"))))),
            )],
        },
        MutualInductive {
            name: "ForestA".to_string(),
            sort: pi("A", ty0(), ty0()),
            num_params: 1,
            constructors: vec![
                ("fnilA".to_string(), pi("A", ty0(), app(g("ForestA"), v("A")))),
                (
                    "fconsA".to_string(),
                    // Π(A). TreeA A → ForestA A → ForestA A
                    pi(
                        "A",
                        ty0(),
                        arrow(
                            app(g("TreeA"), v("A")),
                            arrow(app(g("ForestA"), v("A")), app(g("ForestA"), v("A"))),
                        ),
                    ),
                ),
            ],
        },
    ]
}

#[test]
fn parametric_mutual_block_derives_a_two_kernel_recursor() {
    let mut ctx = std_ctx();
    ctx.add_mutual_inductives(&tree_forest_a_block()).expect("parametric block registers");
    for member in ["TreeA", "ForestA"] {
        let (ty, rec) = derive_recursor(&ctx, member).expect("parametric mutual recursor derives");
        assert!(infer_type(&ctx, &rec).is_ok(), "{member}_rec type-checks: {ty}");
        // The recursor opens with the shared parameter `A0`, then the motives.
        if let Term::Pi { param, .. } = &ty {
            assert_eq!(param, "A0", "{member}_rec's first binder is the shared parameter");
        } else {
            panic!("recursor type must be a Π");
        }
        match double_check(&ctx, &rec) {
            DoubleCheck::Agreed => {}
            other => panic!("both kernels must certify {member}_rec, got {other:?}"),
        }
    }
}

#[test]
fn non_terminating_mutual_fixpoint_is_rejected() {
    // THE MUTUAL GUARD FENCE: `isEven` recursing via `isOdd n` — the SAME parameter,
    // not its predecessor — is a non-decreasing mutual loop that would let `isEven 0`
    // diverge. The mutual Giménez guard must reject it in BOTH kernels (soundness-
    // critical: an accepted non-terminating fixpoint inhabits any type).
    let ctx = std_ctx();
    let bad = is_even_odd(0, true);
    assert!(
        infer_type(&ctx, &bad).is_err(),
        "a mutual call on the un-decremented parameter must be rejected by the main guard"
    );
    assert!(
        recheck(&ctx, &bad).is_err(),
        "the independent re-checker's mutual guard must reject it too"
    );
}

// --- K3b: the mutual guard is TWO-KERNEL (both guards agree) -----------------

#[test]
fn mutual_fixpoint_is_two_kernel_verified() {
    // The independent de Bruijn re-checker must ALSO accept the mutual fixpoint and infer
    // a definitionally-equal type — the two-kernel guarantee (its own copy of the mutual
    // Giménez guard, over de Bruijn levels rather than names).
    let ctx = std_ctx();
    for index in [0, 1] {
        match double_check(&ctx, &is_even_odd(index, false)) {
            DoubleCheck::Agreed => {}
            other => panic!("both kernels must agree on is_even_odd({index}), got {other:?}"),
        }
    }
}

#[test]
fn tree_forest_recursor_is_two_kernel_verified() {
    // The auto-derived mutual recursor — a `MutualFix` whose bodies cross-call the
    // sibling on sub-structures — must pass the INDEPENDENT re-checker's guard too.
    let mut ctx = std_ctx();
    ctx.add_mutual_inductives(&tree_forest_block()).expect("Tree/Forest registers");
    for member in ["Tree", "Forest"] {
        let (_ty, rec) = derive_recursor(&ctx, member).expect("recursor derives");
        match double_check(&ctx, &rec) {
            DoubleCheck::Agreed => {}
            other => panic!("both kernels must agree on {member}_rec, got {other:?}"),
        }
    }
}
