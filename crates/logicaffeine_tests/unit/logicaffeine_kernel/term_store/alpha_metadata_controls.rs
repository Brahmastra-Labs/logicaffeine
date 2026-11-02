use super::*;
#[test]
fn incompatible_alpha_metadata_avoids_canonicalizing_either_dag() {
    for kind in 0..3 {
        let mut s = TermStore::new();
        let ty = s.mk_global("N");
        let x = s.mk_var("x");
        let y = s.mk_var("y");
        let f = s.mk_global("f");
        let g = s.mk_global("g");
        let fx = s.mk_app(f, x);
        let fy = s.mk_app(f, y);
        let gy = s.mk_app(g, y);
        let a = s.mk_lambda("x", ty, fx);
        let b = match kind {
            0 => s.mk_lambda("y", ty, gy),
            1 => {
                let nested = s.mk_app(f, fy);
                s.mk_lambda("y", ty, nested)
            }
            _ => {
                let pair = s.mk_app(f, y);
                let wider = s.mk_app(pair, y);
                s.mk_lambda("y", ty, wider)
            }
        };
        assert!(!s.alpha_same(a, b));
        assert!(
            s.alpha_cache.is_empty(),
            "known alpha-invariant differences need no whole-DAG canonicalization"
        );
        assert!(s.canon_cache.is_empty());
    }
}
#[test]
fn shared_closed_binders_ignore_irrelevant_outer_environments() {
    let mut s = TermStore::new();
    let ty = s.mk_global("N");
    let z = s.mk_var("z");
    let shared = s.mk_lambda("z", ty, z);
    let a = s.mk_lambda("x", ty, shared);
    let b = s.mk_lambda("y", ty, shared);
    let root = s.mk_app(a, b);
    s.alpha_id(root);
    let visits = s
        .canon_cache
        .keys()
        .filter(|(_, level, id)| *level == 1 && *id == shared)
        .count();
    assert_eq!(
        visits, 1,
        "one shared closed subtree at one level needs one canonicalization"
    );
    assert!(s.alpha_same(a, b));
}

#[test]
fn relevant_bindings_shadowing_and_free_names_stay_distinct() {
    let mut s = TermStore::new();
    let ty = s.mk_global("N");
    let mut nested = |outer: &str, inner: &str, occurrence: &str| {
        let v = s.mk_var(occurrence);
        let inner = s.mk_lambda(inner, ty, v);
        s.mk_lambda(outer, ty, inner)
    };
    let terms = [
        nested("x", "y", "x"),
        nested("a", "b", "a"),
        nested("x", "x", "x"),
        nested("a", "b", "b"),
        nested("x", "z", "free"),
        nested("y", "z", "free"),
        nested("y", "z", "x"),
    ];
    for (a, b, equal) in [
        (0, 1, true),
        (2, 3, true),
        (0, 2, false),
        (4, 5, true),
        (0, 6, false),
        (4, 6, false),
    ] {
        assert_eq!(s.alpha_same(terms[a], terms[b]), equal);
        assert_eq!(s.alpha_same(terms[b], terms[a]), equal);
    }
}
#[test]
fn closed_shared_subtrees_keep_their_absolute_binder_level() {
    let mut s = TermStore::new();
    let ty = s.mk_global("N");
    let z = s.mk_var("z");
    let shared = s.mk_lambda("z", ty, z);
    let at_one = s.mk_lambda("x", ty, shared);
    let inner = s.mk_lambda("y", ty, shared);
    let at_two = s.mk_lambda("x", ty, inner);
    let root = s.mk_app(at_one, at_two);
    s.alpha_id(root);
    let one = s.canonicalise(SubstId::EMPTY, 1, shared);
    let two = s.canonicalise(SubstId::EMPTY, 2, shared);
    assert_ne!(
        one, two,
        "closed binders still introduce different absolute levels"
    );
    assert!(s.canon_cache.contains_key(&(SubstId::EMPTY, 1, shared)));
    assert!(s.canon_cache.contains_key(&(SubstId::EMPTY, 2, shared)));
    let source_canonical = s.alpha_id(at_one);
    assert!(s.alpha_same(at_one, source_canonical));
}
#[test]
fn saturated_sizes_only_reject_inequality_and_never_certify_equality() {
    let mut s = TermStore::new();
    let mut a = s.mk_global("A");
    let mut b = s.mk_global("B");
    for _ in 0..70 {
        a = s.mk_app(a, a);
        b = s.mk_app(b, b);
    }
    assert_eq!(s.info[a.index()].tree_size, u64::MAX);
    assert_eq!(s.info[b.index()].tree_size, u64::MAX);
    assert!(!s.alpha_same(a, b));
    assert!(s.alpha_cache.is_empty());
    let ty = s.mk_global("N");
    let x = s.mk_var("x");
    let y = s.mk_var("y");
    let free = s.mk_var("free");
    let a = s.mk_lambda("x", ty, x);
    let b = s.mk_lambda("y", ty, y);
    let different = s.mk_lambda("y", ty, free);
    assert!(s.alpha_same(a, b));
    assert!(!s.alpha_same(a, different));
}
#[test]
fn equal_shapes_and_global_sets_still_check_universe_arguments() {
    let mut s = TermStore::new();
    let a = s.mk_const("poly", &[Universe::Type(0)]);
    let b = s.mk_const("poly", &[Universe::Type(1)]);
    assert_eq!(s.info[a.index()].consts, s.info[b.index()].consts);
    assert_eq!(s.info[a.index()].tree_size, s.info[b.index()].tree_size);
    assert!(!s.alpha_same(a, b));
}
