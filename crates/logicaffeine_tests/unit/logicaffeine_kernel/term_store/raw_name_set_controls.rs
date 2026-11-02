use super::*;
#[test]
fn raw_metadata_admission_counts_the_existing_inventory_without_mutation() {
    let mut store = TermStore::new();
    let before = store.sets.len();
    assert!(before > 0);
    assert!(matches!(
        store.admit_name_set_growth(u32::MAX as u64),
        Err(StoreOperationLimit::Nodes)
    ));
    assert_eq!(store.sets.len(), before);
    let _ = store.mk_var("x");
    assert!(store.sets.len() > before);
    let remaining = u32::MAX as u64 - store.sets.len() as u64;
    assert!(store.admit_name_set_growth(remaining).is_ok());
    assert!(matches!(
        store.admit_name_set_growth(remaining + 1),
        Err(StoreOperationLimit::Nodes)
    ));
}
