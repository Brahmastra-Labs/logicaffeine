use super::*;
#[test]
fn name_set_inventory_admission_rejects_dense_id_exhaustion() {
    let ceiling = u32::MAX as usize;
    assert!(TermStore::admit_name_set_inventory(ceiling - 1, 1).is_ok());
    assert!(matches!(
        TermStore::admit_name_set_inventory(ceiling, 1),
        Err(StoreOperationLimit::Nodes)
    ));
    assert!(matches!(
        TermStore::admit_name_set_inventory(1, u64::MAX),
        Err(StoreOperationLimit::Nodes)
    ));
}
#[test]
fn bounded_substitution_preserves_capture_avoidance_through_fresh_metadata() {
    let mut store = TermStore::new();
    let x = store.mk_var("x");
    let y = store.mk_var("y");
    let kind = store.mk_sort(&Universe::Type(0));
    let sx = store.symbol("x");
    let sy = store.symbol("y");
    let root = store.add(Node::Lambda {
        param: sy,
        param_type: kind,
        body: x,
    });
    let mut work = 1_000_000;
    let mut allocation = 1_000_000;
    let result = store
        .instantiate_bounded(root, &[(sx, y)], &mut work, 1_000, &mut allocation)
        .unwrap();
    assert_eq!(store.free_vars(result), vec!["y"]);
    let Node::Lambda { param, body, .. } = store.node(result) else {
        panic!("expected lambda")
    };
    assert_ne!(*param, sy);
    assert!(matches!(store.node(*body),Node::Var(name) if *name==sy));
}
