use super::*;
fn budget() -> RecursorBudget {
    RecursorBudget {
        max_depth: 128,
        max_tree_nodes: 100_000,
        max_store_nodes: 100_000,
        work: u64::MAX,
        allocation: u64::MAX,
        materialized_nodes: 100_000,
    }
}
#[test]
fn wide_binder_free_raw_input_does_not_pay_a_quadratic_name_set_inventory() {
    let mut limits = budget();
    let mut construction = Construction::new(&mut limits).unwrap();
    // The old suggested blanket N^2 inventory bound would exceed u32 on this shallow
    // input. Actual metadata creation is linear in the number of children.
    let term = Term::Match {
        discriminant: Box::new(Term::Hole),
        motive: Box::new(Term::Hole),
        cases: vec![Term::Hole; 65_536],
    };
    let id = construction.intern(&term).unwrap();
    assert_eq!(construction.store.tree_size(id), 65_539);
    assert_eq!(construction.store.len(), 2);
}
#[test]
fn mutual_binder_cross_product_is_admitted_before_raw_intern() {
    // Exercise the exact nonallocating estimate/guard boundary without executing an
    // intentionally enormous pre-fix metadata allocation as a RED test.
    let term = Term::MutualFix {
        defs: (0..65_536).map(|_| ("x".into(), Term::Hole)).collect(),
        index: 0,
    };
    let growth = raw_metadata_set_growth(&term);
    assert!(growth > u32::MAX as u64);
    let store = TermStore::new();
    assert!(matches!(
        store.admit_name_set_growth(growth),
        Err(crate::term_store::StoreOperationLimit::Nodes)
    ));
}
