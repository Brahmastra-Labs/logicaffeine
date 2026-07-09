//! Nested inductives (K3c) — a rose tree `RTree := rnode : TList RTree → RTree`, where
//! `RTree` recurs NESTED inside the generic container `TList`. The compiler specializes
//! `TList` into a mutual sibling and emits conversion isos; the kernel checks everything.
//! The isos are the careful part: they must type-check AND genuinely round-trip.

use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{
    derive_recursor, double_check, infer_type, is_subtype, normalize, Context, DoubleCheck,
    NestedDecl, Term, Universe,
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
    Term::Pi { param: "_".to_string(), param_type: Box::new(a), body_type: Box::new(b) }
}
fn v(n: &str) -> Term {
    Term::Var(n.to_string())
}
fn pi(p: &str, t: Term, b: Term) -> Term {
    Term::Pi { param: p.to_string(), param_type: Box::new(t), body_type: Box::new(b) }
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
        constructors: vec![("rnode".to_string(), arrow(app(g("TList"), g("RTree")), g("RTree")))],
    }
}

#[test]
fn rose_tree_via_tlist_compiles_and_typechecks() {
    let mut ctx = std_ctx();
    let info = ctx.add_nested_inductive(&rose_tree_decl()).expect("rose tree compiles");
    // `RTree`, its constructor, and the specialized sibling are all registered and check.
    assert!(infer_type(&ctx, &g("RTree")).is_ok(), "RTree registered");
    assert!(infer_type(&ctx, &g("rnode")).is_ok(), "rnode registered");
    assert_eq!(info.siblings, vec!["RTree$TList".to_string()], "TList specialized into a sibling");
    assert!(infer_type(&ctx, &g("RTree$TList")).is_ok(), "the specialized sibling is registered");
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
    let info = ctx.add_nested_inductive(&rose_tree_decl()).expect("rose tree compiles");
    let iso = &info.isos[0];
    // Both conversions were type-checked as part of registration; confirm their types and
    // that the INDEPENDENT re-checker agrees (the isos are ordinary certified terms).
    let to_ty = infer_type(&ctx, &g(&iso.to_generic)).expect("to-iso registered");
    let from_ty = infer_type(&ctx, &g(&iso.from_generic)).expect("from-iso registered");
    assert!(
        is_subtype(&ctx, &to_ty, &arrow(g("RTree$TList"), app(g("TList"), g("RTree")))),
        "to_generic : RTree$TList → TList RTree, got {to_ty}"
    );
    assert!(
        is_subtype(&ctx, &from_ty, &arrow(app(g("TList"), g("RTree")), g("RTree$TList"))),
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
    ctx.add_nested_inductive(&rose_tree_decl()).expect("rose tree compiles");
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
    let info = ctx.add_nested_inductive(&deep_tree_decl()).expect("deep tree compiles");
    assert!(infer_type(&ctx, &g("DTree")).is_ok(), "DTree registered");
    assert!(infer_type(&ctx, &g("dnode")).is_ok(), "dnode registered");
    assert_eq!(
        info.siblings,
        vec!["DTree$TList".to_string(), "DTree$TList$TList".to_string()],
        "both nested levels specialized, inner first"
    );
    assert!(infer_type(&ctx, &g("DTree$TList")).is_ok(), "inner sibling registered");
    assert!(infer_type(&ctx, &g("DTree$TList$TList")).is_ok(), "outer sibling registered");
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
    let info = ctx.add_nested_inductive(&deep_tree_decl()).expect("deep tree compiles");
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
    ctx.add_nested_inductive(&deep_tree_decl()).expect("deep tree compiles");
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
    let info = ctx.add_nested_inductive(&deep_tree_decl()).expect("deep tree compiles");
    // The outer iso pair is the second (isos are inner-first).
    let outer = &info.isos[1];
    assert_eq!(outer.sibling, "DTree$TList$TList");
    let to_b = g(&outer.to_generic);
    let from_b = g(&outer.from_generic);

    // sb = SB_TCons SA_TNil SB_TNil : DTree$TList$TList
    let sa_nil = g("DTree$TList_TNil");
    let sb = apps(g("DTree$TList$TList_TCons"), &[sa_nil, g("DTree$TList$TList_TNil")]);
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
        constructors: vec![("bnode".to_string(), arrow(app(g("TList"), g("BigTree")), g("BigTree")))],
    };
    assert!(ctx.add_nested_inductive(&decl).is_err(), "a non-Type-0 nested inductive must be rejected");
}

#[test]
fn nesting_in_an_impure_container_is_rejected() {
    // AUDIT FIX: only PURE containers (every field the element or a recursive occurrence)
    // are specialized soundly. A container with an extra carried field could smuggle a
    // higher-universe type into the `Type 0` sibling, so it is refused fail-closed.
    let mut ctx = std_ctx();
    // LList with `LCons : Π(A). A → Nat → LList A → LList A` — the `Nat` field is impure.
    ctx.add_inductive("LList", arrow(ty0(), ty0()));
    ctx.add_constructor("LNil", "LList", pi("A", ty0(), app(g("LList"), v("A"))));
    ctx.add_constructor(
        "LCons",
        "LList",
        pi(
            "A",
            ty0(),
            arrow(v("A"), arrow(g("Nat"), arrow(app(g("LList"), v("A")), app(g("LList"), v("A"))))),
        ),
    );
    let decl = NestedDecl {
        name: "LTree".to_string(),
        sort: ty0(),
        constructors: vec![("lnode".to_string(), arrow(app(g("LList"), g("LTree")), g("LTree")))],
    };
    assert!(
        ctx.add_nested_inductive(&decl).is_err(),
        "nesting in a container with a non-element/non-recursive field must be rejected"
    );
}

#[test]
fn rose_tree_isos_round_trip_by_computation() {
    // THE CAREFUL PART: the conversions must genuinely invert. Build a specialized value
    // `RTree$TList_TCons t RTree$TList_TNil` (a one-element spine), send it through
    // `from_generic ∘ to_generic`, and check by REDUCTION it returns unchanged — and dually
    // for a generic `TList RTree` value through `to_generic ∘ from_generic`.
    let mut ctx = std_ctx();
    let info = ctx.add_nested_inductive(&rose_tree_decl()).expect("rose tree compiles");
    let iso = &info.isos[0];
    let to = g(&iso.to_generic);
    let from = g(&iso.from_generic);

    // A leaf `RTree`: rnode (empty specialized list).
    let leaf = app(g("rnode"), g("RTree$TList_TNil"));
    // sib = RTree$TList_TCons leaf RTree$TList_TNil : RTree$TList
    let sib = apps(g("RTree$TList_TCons"), &[leaf.clone(), g("RTree$TList_TNil")]);
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
