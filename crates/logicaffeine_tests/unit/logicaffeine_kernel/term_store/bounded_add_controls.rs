use super::*;
#[test]
fn bounded_add_refuses_before_store_mutation_when_metadata_allocation_is_exhausted() {
    let mut store = TermStore::new();
    let x = store.mk_var("x");
    let before = store.len();
    let mut work = 100_000;
    let mut allocation = 0;
    assert!(matches!(
        store.add_bounded(Node::App(x, x), &mut work, 100, &mut allocation),
        Err(StoreOperationLimit::Allocation)
    ));
    assert_eq!(store.len(), before);
}
#[test]
fn bounded_add_prices_shared_metadata_without_unfolding_exponential_terms() {
    let mut store = TermStore::new();
    let mut value = store.mk_var("x");
    for _ in 0..30 {
        value = store.mk_app(value, value);
    }
    assert!(store.tree_size(value) > 1_000_000_000);
    let mut work = 2_000;
    let mut allocation = 2_000;
    let added = store
        .add_bounded(Node::App(value, value), &mut work, 100, &mut allocation)
        .unwrap();
    assert_eq!(store.free_vars(added), vec!["x"]);
}
#[test]
fn bounded_add_handles_wide_identical_child_sets_with_linear_admission() {
    let mut store = TermStore::new();
    let x = store.mk_var("x");
    let node = Node::Match {
        discriminant: x,
        motive: x,
        cases: vec![x; 1_000].into_boxed_slice(),
    };
    let mut work = 100_000;
    let mut allocation = 100_000;
    let added = store
        .add_bounded(node, &mut work, 10, &mut allocation)
        .unwrap();
    assert_eq!(store.free_vars(added), vec!["x"]);
}
#[test]
fn bounded_add_dedup_hit_needs_no_new_node_or_allocation_allowance() {
    let mut store = TermStore::new();
    let x = store.mk_var("x");
    let prior = store.mk_app(x, x);
    let mut work = 100;
    let mut allocation = 0;
    let ceiling = store.len();
    assert_eq!(
        store
            .add_bounded(Node::App(x, x), &mut work, ceiling, &mut allocation)
            .unwrap(),
        prior
    );
}
#[test]
fn bounded_add_admits_scoped_metadata_before_a_mutual_fix() {
    let mut store = TermStore::new();
    let x = store.mk_var("x");
    let y = store.mk_var("y");
    let sx = store.symbol("x");
    let sy = store.symbol("y");
    let node = Node::MutualFix {
        defs: vec![(sx, y), (sy, x)].into_boxed_slice(),
        index: 0,
    };
    let mut work = 10_000;
    let mut allocation = 10_000;
    let added = store
        .add_bounded(node, &mut work, 100, &mut allocation)
        .unwrap();
    assert!(store.free_vars(added).is_empty());
}
