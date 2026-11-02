//! Nested inductives (K3c) — a rose tree `RTree := rnode : TList RTree → RTree`, where
//! `RTree` recurs NESTED inside the generic container `TList`. The compiler specializes
//! `TList` into a mutual sibling and emits conversion isos; the kernel checks everything.
//! The isos are the careful part: they must type-check AND genuinely round-trip.

use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{
    derive_recursor, double_check, infer_type, is_subtype, normalize, Context, DoubleCheck,
    KernelError, NestedDecl, Term, Universe,
};

fn g(n: &str) -> Term {
    Term::Global(n.to_string())
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn apps(f: Term, xs: &[Term]) -> Term {
    xs.iter().fold(f, |a, x| app(a, x.clone()))
}
fn arrow(a: Term, b: Term) -> Term {
    Term::Pi {
        param: "_".to_string(),
        param_type: Box::new(a),
        body_type: Box::new(b),
    }
}
fn v(n: &str) -> Term {
    Term::Var(n.to_string())
}
fn pi(p: &str, t: Term, b: Term) -> Term {
    Term::Pi {
        param: p.to_string(),
        param_type: Box::new(t),
        body_type: Box::new(b),
    }
}
fn ty0() -> Term {
    Term::Sort(Universe::Type(0))
}
fn std_ctx() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx
}

/// `RTree := rnode : TList RTree → RTree` — a rose tree over the generic `TList`.
fn rose_tree_decl() -> NestedDecl {
    NestedDecl {
        name: "RTree".to_string(),
        sort: ty0(),
        constructors: vec![(
            "rnode".to_string(),
            arrow(app(g("TList"), g("RTree")), g("RTree")),
        )],
    }
}

#[test]
fn rose_tree_via_tlist_compiles_and_typechecks() {
    let mut ctx = std_ctx();
    let info = ctx
        .add_nested_inductive(&rose_tree_decl())
        .expect("rose tree compiles");
    // `RTree`, its constructor, and the specialized sibling are all registered and check.
    assert!(infer_type(&ctx, &g("RTree")).is_ok(), "RTree registered");
    assert!(infer_type(&ctx, &g("rnode")).is_ok(), "rnode registered");
    assert_eq!(
        info.siblings,
        vec!["RTree$TList".to_string()],
        "TList specialized into a sibling"
    );
    assert!(
        infer_type(&ctx, &g("RTree$TList")).is_ok(),
        "the specialized sibling is registered"
    );
    // `rnode` now takes the SPECIALIZED sibling — `RTree$TList → RTree`.
    let rnode_ty = infer_type(&ctx, &g("rnode")).unwrap();
    assert!(
        is_subtype(&ctx, &rnode_ty, &arrow(g("RTree$TList"), g("RTree"))),
        "rnode : RTree$TList → RTree, got {rnode_ty}"
    );
}

#[test]
fn rose_tree_isos_are_kernel_checked_and_two_kernel() {
    let mut ctx = std_ctx();
    let info = ctx
        .add_nested_inductive(&rose_tree_decl())
        .expect("rose tree compiles");
    let iso = &info.isos[0];
    // Both conversions were type-checked as part of registration; confirm their types and
    // that the INDEPENDENT re-checker agrees (the isos are ordinary certified terms).
    let to_ty = infer_type(&ctx, &g(&iso.to_generic)).expect("to-iso registered");
    let from_ty = infer_type(&ctx, &g(&iso.from_generic)).expect("from-iso registered");
    assert!(
        is_subtype(
            &ctx,
            &to_ty,
            &arrow(g("RTree$TList"), app(g("TList"), g("RTree")))
        ),
        "to_generic : RTree$TList → TList RTree, got {to_ty}"
    );
    assert!(
        is_subtype(
            &ctx,
            &from_ty,
            &arrow(app(g("TList"), g("RTree")), g("RTree$TList"))
        ),
        "from_generic : TList RTree → RTree$TList, got {from_ty}"
    );
    for name in [&iso.to_generic, &iso.from_generic] {
        match double_check(&ctx, &g(name)) {
            DoubleCheck::Agreed => {}
            other => panic!("both kernels must certify iso {name}, got {other:?}"),
        }
    }
}

#[test]
fn rose_tree_gets_a_working_two_kernel_recursor() {
    // The payoff of the specialization: the nested inductive gets a genuine induction /
    // recursion principle (the mutual recursor over `[RTree, RTree$TList]`), which recurses
    // through the specialized list. It must derive and be certified by BOTH kernels — so a
    // rose tree is fully eliminable, exactly like a hand-written mutual type.
    let mut ctx = std_ctx();
    ctx.add_nested_inductive(&rose_tree_decl())
        .expect("rose tree compiles");
    let (_ty, rtree_rec) = derive_recursor(&ctx, "RTree").expect("RTree_rec derives");
    match double_check(&ctx, &rtree_rec) {
        DoubleCheck::Agreed => {}
        other => panic!("both kernels must certify RTree_rec, got {other:?}"),
    }
}

/// `DTree := dnode : TList (TList DTree) → DTree` — `DTree` nested TWO container levels deep.
fn deep_tree_decl() -> NestedDecl {
    NestedDecl {
        name: "DTree".to_string(),
        sort: ty0(),
        constructors: vec![(
            "dnode".to_string(),
            arrow(app(g("TList"), app(g("TList"), g("DTree"))), g("DTree")),
        )],
    }
}

#[test]
fn deeper_two_level_nesting_compiles_and_typechecks() {
    // Deeper nesting `TList (TList DTree)` is compiled by RECURSIVE specialization: the inner
    // `TList DTree` becomes sibling `DTree$TList`, and the outer `TList (that)` becomes sibling
    // `DTree$TList$TList`, mutually with `DTree`. Both siblings register and `dnode` takes the
    // OUTERMOST sibling.
    let mut ctx = std_ctx();
    let info = ctx
        .add_nested_inductive(&deep_tree_decl())
        .expect("deep tree compiles");
    assert!(infer_type(&ctx, &g("DTree")).is_ok(), "DTree registered");
    assert!(infer_type(&ctx, &g("dnode")).is_ok(), "dnode registered");
    assert_eq!(
        info.siblings,
        vec!["DTree$TList".to_string(), "DTree$TList$TList".to_string()],
        "both nested levels specialized, inner first"
    );
    assert!(
        infer_type(&ctx, &g("DTree$TList")).is_ok(),
        "inner sibling registered"
    );
    assert!(
        infer_type(&ctx, &g("DTree$TList$TList")).is_ok(),
        "outer sibling registered"
    );
    let dnode_ty = infer_type(&ctx, &g("dnode")).unwrap();
    assert!(
        is_subtype(&ctx, &dnode_ty, &arrow(g("DTree$TList$TList"), g("DTree"))),
        "dnode : DTree$TList$TList → DTree, got {dnode_ty}"
    );
}

#[test]
fn deeper_nesting_isos_are_two_kernel_certified() {
    // Both levels of isos are ordinary certified terms; the OUTER iso applies the INNER iso on
    // its element fields, so it can only check once the inner iso is registered (inner-first).
    let mut ctx = std_ctx();
    let info = ctx
        .add_nested_inductive(&deep_tree_decl())
        .expect("deep tree compiles");
    assert_eq!(info.isos.len(), 2, "one iso pair per nested level");
    for iso in &info.isos {
        for name in [&iso.to_generic, &iso.from_generic] {
            assert!(infer_type(&ctx, &g(name)).is_ok(), "iso {name} registered");
            match double_check(&ctx, &g(name)) {
                DoubleCheck::Agreed => {}
                other => panic!("both kernels must certify iso {name}, got {other:?}"),
            }
        }
    }
}

#[test]
fn deeper_nesting_gets_a_working_recursor() {
    // The payoff: a doubly-nested inductive still gets a genuine induction principle (a
    // 3-motive mutual recursor over `[DTree, DTree$TList, DTree$TList$TList]`), certified by
    // BOTH kernels — fully eliminable, exactly like a hand-written triple-mutual type.
    let mut ctx = std_ctx();
    ctx.add_nested_inductive(&deep_tree_decl())
        .expect("deep tree compiles");
    let (_ty, rec) = derive_recursor(&ctx, "DTree").expect("DTree_rec derives");
    match double_check(&ctx, &rec) {
        DoubleCheck::Agreed => {}
        other => panic!("both kernels must certify DTree_rec, got {other:?}"),
    }
}

#[test]
fn deeper_nesting_isos_round_trip_by_computation() {
    // THE CAREFUL PART for deeper nesting: build the specialized value
    // `SB_TCons (SA_TNil) (SB_TNil)` (a one-element outer list whose single element is an empty
    // inner list), send it through `from_B ∘ to_B`, and check by REDUCTION it is unchanged —
    // the outer iso must correctly delegate to the inner iso on the element.
    let mut ctx = std_ctx();
    let info = ctx
        .add_nested_inductive(&deep_tree_decl())
        .expect("deep tree compiles");
    // The outer iso pair is the second (isos are inner-first).
    let outer = &info.isos[1];
    assert_eq!(outer.sibling, "DTree$TList$TList");
    let to_b = g(&outer.to_generic);
    let from_b = g(&outer.from_generic);

    // sb = SB_TCons SA_TNil SB_TNil : DTree$TList$TList
    let sa_nil = g("DTree$TList_TNil");
    let sb = apps(
        g("DTree$TList$TList_TCons"),
        &[sa_nil, g("DTree$TList$TList_TNil")],
    );
    let round = app(from_b.clone(), app(to_b.clone(), sb.clone()));
    assert_eq!(
        normalize(&ctx, &round),
        normalize(&ctx, &sb),
        "from_B (to_B sb) = sb through the nested element iso"
    );

    // And dually a generic value `TCons (TList DTree) (TNil DTree) (TNil (TList DTree))`.
    let gen = apps(
        g("TCons"),
        &[
            app(g("TList"), g("DTree")),
            app(g("TNil"), g("DTree")),
            app(g("TNil"), app(g("TList"), g("DTree"))),
        ],
    );
    let round_gen = app(to_b, app(from_b, gen.clone()));
    assert_eq!(
        normalize(&ctx, &round_gen),
        normalize(&ctx, &gen),
        "to_B (from_B gen) = gen"
    );
}

#[test]
fn higher_universe_nested_inductive_is_rejected() {
    // AUDIT FIX (universe safety): the specialized siblings are emitted at `Type 0`, so a
    // non-`Type 0` nested inductive must be rejected fail-closed rather than silently
    // registering a higher-universe field inside a `Type 0` sibling.
    let mut ctx = std_ctx();
    let decl = NestedDecl {
        name: "BigTree".to_string(),
        sort: Term::Sort(Universe::Type(1)),
        constructors: vec![(
            "bnode".to_string(),
            arrow(app(g("TList"), g("BigTree")), g("BigTree")),
        )],
    };
    assert!(
        ctx.add_nested_inductive(&decl).is_err(),
        "a non-Type-0 nested inductive must be rejected"
    );
}

/// A container whose constructor CARRIES a closed extra field (the `Nat` in `LCons`) is nested
/// through soundly: the field rides through the mirror in its own slot, exactly as Lean 4.14
/// accepts `inductive LTree | lnode : LList LTree → LTree` over this container (run on this box;
/// see the `inductive_compile` module header). The hazard the old fence stood for — a carried
/// field smuggling a higher-universe type into the `Type 0` sibling — is refused by the trusted
/// mutual door, transactionally.
#[test]
fn a_closed_carried_field_nests_and_an_oversized_one_is_refused() {
    let list_like = |ctx: &mut Context, name: &str, carried: Term| {
        ctx.add_inductive(name, arrow(ty0(), ty0()));
        ctx.add_constructor(
            &format!("{name}Nil"),
            name,
            pi("A", ty0(), app(g(name), v("A"))),
        );
        ctx.add_constructor(
            &format!("{name}Cons"),
            name,
            pi(
                "A",
                ty0(),
                arrow(
                    v("A"),
                    arrow(carried, arrow(app(g(name), v("A")), app(g(name), v("A")))),
                ),
            ),
        );
    };
    let tree_through = |tree: &str, container: &str| NestedDecl {
        name: tree.to_string(),
        sort: ty0(),
        constructors: vec![(
            format!("{tree}_node"),
            arrow(app(g(container), g(tree)), g(tree)),
        )],
    };

    // LList with `LCons : Π(A). A → Nat → LList A → LList A` — a closed carried `Nat`.
    let mut ctx = std_ctx();
    list_like(&mut ctx, "LList", g("Nat"));
    let info = ctx
        .add_nested_inductive(&tree_through("LTree", "LList"))
        .expect("a closed `Nat` field rides through the mirror, as Lean accepts it");
    assert_eq!(
        info.siblings,
        vec!["LTree$LList".to_string()],
        "one container level, one sibling"
    );
    for iso in &info.isos {
        for name in [&iso.to_generic, &iso.from_generic] {
            match double_check(&ctx, &g(name)) {
                DoubleCheck::Agreed => {}
                other => panic!("both kernels must certify iso {name}, got {other:?}"),
            }
        }
    }

    // THE HAZARD: a carried field whose own sort (`Type 0 → Nat : Type 1`) exceeds the `Type 0`
    // sibling it would be stored in. Refused by the checked door, and nothing is registered.
    let mut hazard = std_ctx();
    list_like(&mut hazard, "HList", arrow(ty0(), g("Nat")));
    let refusal = hazard
        .add_nested_inductive(&tree_through("HTree", "HList"))
        .expect_err("a carried field above the sibling's sort must be refused");
    assert!(
        refusal.to_string().contains("universe inconsistency"),
        "the refusal names the universe violation: {refusal}"
    );
    assert!(
        !hazard.is_inductive("HTree") && hazard.get_global("HTree_node").is_none(),
        "a refused nesting registers nothing"
    );
}

/// A list-like container `name : Type → Type` whose cons carries `carried` beside the element.
fn carrying_list(ctx: &mut Context, name: &str, carried: Term) {
    ctx.add_inductive(name, arrow(ty0(), ty0()));
    ctx.add_constructor(
        &format!("{name}Nil"),
        name,
        pi("A", ty0(), app(g(name), v("A"))),
    );
    ctx.add_constructor(
        &format!("{name}Cons"),
        name,
        pi(
            "A",
            ty0(),
            arrow(
                v("A"),
                arrow(carried, arrow(app(g(name), v("A")), app(g(name), v("A")))),
            ),
        ),
    );
}
fn nested_through(tree: &str, container: &str) -> NestedDecl {
    NestedDecl {
        name: tree.to_string(),
        sort: ty0(),
        constructors: vec![(
            format!("{tree}_node"),
            arrow(app(g(container), g(tree)), g(tree)),
        )],
    }
}
fn registers_nothing(ctx: &Context, tree: &str) {
    assert!(
        !ctx.is_inductive(tree) && ctx.get_global(&format!("{tree}_node")).is_none(),
        "a refused nesting of `{tree}` registers nothing"
    );
}

/// The neighbours of the accepted closed carried field, decided as Lean 4.14 decides them (run
/// on this box: `inductive TNeg | node : CNeg TNeg → TNeg` over `cons : A → (A → Nat) → CNeg A →
/// CNeg A` is "(kernel) arg #2 … has a non positive occurrence", while a carried proof `0 = 0` is
/// accepted). A carried field that puts the ELEMENT in a domain is refused as a positivity
/// violation, one that names a type not yet declared is refused, and a carried closed PROOF nests
/// through with both kernels certifying the isos.
#[test]
fn the_neighbours_of_a_closed_carried_field_are_decided_as_lean_decides_them() {
    let mut negative = std_ctx();
    carrying_list(&mut negative, "NegList", arrow(v("A"), g("Nat")));
    match negative.add_nested_inductive(&nested_through("NegTree", "NegList")) {
        Err(KernelError::PositivityViolation { .. }) => {}
        other => panic!(
            "a carried field with the element in a domain is a positivity violation: {other:?}"
        ),
    }
    registers_nothing(&negative, "NegTree");

    let mut pending = std_ctx();
    carrying_list(&mut pending, "PendList", g("NotYetDeclared"));
    assert!(
        pending
            .add_nested_inductive(&nested_through("PendTree", "PendList"))
            .is_err(),
        "a carried field naming an undeclared type is refused"
    );
    registers_nothing(&pending, "PendTree");

    let mut proof = std_ctx();
    carrying_list(
        &mut proof,
        "ProofList",
        apps(g("Eq"), &[g("Nat"), g("Zero"), g("Zero")]),
    );
    let info = proof
        .add_nested_inductive(&nested_through("ProofTree", "ProofList"))
        .expect("a carried closed proof nests through, as Lean accepts it");
    assert_eq!(info.siblings, vec!["ProofTree$ProofList".to_string()]);
    for iso in &info.isos {
        for name in [&iso.to_generic, &iso.from_generic] {
            assert_eq!(
                double_check(&proof, &g(name)),
                DoubleCheck::Agreed,
                "iso {name}"
            );
        }
    }
}

#[test]
fn rose_tree_isos_round_trip_by_computation() {
    // THE CAREFUL PART: the conversions must genuinely invert. Build a specialized value
    // `RTree$TList_TCons t RTree$TList_TNil` (a one-element spine), send it through
    // `from_generic ∘ to_generic`, and check by REDUCTION it returns unchanged — and dually
    // for a generic `TList RTree` value through `to_generic ∘ from_generic`.
    let mut ctx = std_ctx();
    let info = ctx
        .add_nested_inductive(&rose_tree_decl())
        .expect("rose tree compiles");
    let iso = &info.isos[0];
    let to = g(&iso.to_generic);
    let from = g(&iso.from_generic);

    // A leaf `RTree`: rnode (empty specialized list).
    let leaf = app(g("rnode"), g("RTree$TList_TNil"));
    // sib = RTree$TList_TCons leaf RTree$TList_TNil : RTree$TList
    let sib = apps(
        g("RTree$TList_TCons"),
        &[leaf.clone(), g("RTree$TList_TNil")],
    );
    let round_sib = app(from.clone(), app(to.clone(), sib.clone()));
    assert_eq!(
        normalize(&ctx, &round_sib),
        normalize(&ctx, &sib),
        "from_generic (to_generic sib) = sib"
    );

    // gen = TCons RTree leaf (TNil RTree) : TList RTree
    let gen = apps(g("TCons"), &[g("RTree"), leaf, app(g("TNil"), g("RTree"))]);
    let round_gen = app(to, app(from, gen.clone()));
    assert_eq!(
        normalize(&ctx, &round_gen),
        normalize(&ctx, &gen),
        "to_generic (from_generic gen) = gen"
    );
}
