use super::*;
fn budget() -> RecursorBudget {
    RecursorBudget {
        max_depth: 128,
        max_tree_nodes: 100_000,
        max_store_nodes: 100_000,
        work: 1_000_000_000_000,
        allocation: 1_000_000_000,
        materialized_nodes: 100_000,
    }
}
#[test]
fn generated_symbol_payload_is_charged_before_extracting_a_leaf() {
    let mut limits = budget();
    let mut construction = Construction::new(&mut limits).unwrap();
    let name = "x".repeat(65_536);
    let root = construction.var(&name).unwrap();
    construction.budget.allocation = 32;
    assert!(matches!(
        construction.extract(root),
        Err(RecursorGenerationError::ResourceLimit)
    ));
}
#[test]
fn admitted_level_payload_remains_visible_to_tree_materialization() {
    let mut limits = budget();
    let mut construction = Construction::new(&mut limits).unwrap();
    let level = Universe::Var("u".repeat(65_536));
    let level = construction.level(&level).unwrap();
    let root = construction.add(Node::Sort(level)).unwrap();
    construction.budget.allocation = 32;
    assert!(matches!(
        construction.extract(root),
        Err(RecursorGenerationError::ResourceLimit)
    ));
}
#[test]
fn generated_level_collection_preserves_first_occurrence_order() {
    let mut limits = budget();
    let mut construction = Construction::new(&mut limits).unwrap();
    let v = construction.level(&Universe::Var("v".into())).unwrap();
    let u = construction.level(&Universe::Var("u".into())).unwrap();
    let v = construction.add(Node::Sort(v)).unwrap();
    let u = construction.add(Node::Sort(u)).unwrap();
    let repeated = construction.app(u, v).unwrap();
    let root = construction.app(v, repeated).unwrap();
    let levels = construction.free_level_vars(root).unwrap();
    assert_eq!(levels, vec!["v", "u"]);
    let tree = construction.extract(root).unwrap();
    assert_eq!(levels, crate::term::free_level_vars(&tree));
}

#[test]
fn distinct_universe_inventory_is_bounded_across_calls_before_mutation() {
    let mut limits = budget();
    limits.max_store_nodes = 3;
    let mut construction = Construction::new(&mut limits).unwrap();
    for n in 0..3 {
        construction.level(&Universe::Type(n)).unwrap();
    }
    assert_eq!(construction.store.level_count(), 3);
    assert!(matches!(
        construction.level(&Universe::Type(3)),
        Err(RecursorGenerationError::ResourceLimit)
    ));
    assert_eq!(construction.store.level_count(), 3);
    assert_eq!(construction.store.len(), 0);
}
#[test]
fn raw_term_intern_checks_shared_universe_inventory_before_mutation() {
    let mut limits = budget();
    limits.max_store_nodes = 3;
    let mut construction = Construction::new(&mut limits).unwrap();
    for n in 0..3 {
        construction.level(&Universe::Type(n)).unwrap();
    }
    assert!(matches!(
        construction.intern(&Term::Sort(Universe::Type(3))),
        Err(RecursorGenerationError::ResourceLimit)
    ));
    assert_eq!(construction.store.level_count(), 3);
    assert_eq!(construction.store.len(), 0);
}
#[test]
fn store_policy_larger_than_id_space_is_rejected_before_construction() {
    if let Some(too_large) = (u32::MAX as usize).checked_add(1) {
        let mut limits = budget();
        limits.max_store_nodes = too_large;
        assert!(matches!(
            Construction::new(&mut limits),
            Err(RecursorGenerationError::ResourceLimit)
        ));
    }
}
#[test]
fn an_interned_node_is_charged_its_own_payload_not_the_whole_trees() {
    let mut limits = budget();
    let mut construction = Construction::new(&mut limits).unwrap();
    // 4,099 nodes and 4,098 name bytes in one term: the envelope of ONE extracted node is still
    // one name plus its node overhead.
    let g = || Term::Global("g".into());
    let wide = Term::Match {
        discriminant: Box::new(g()),
        motive: Box::new(g()),
        cases: vec![g(); 4096],
    };
    construction.intern(&wide).unwrap();
    let leaf = construction.intern(&g()).unwrap();
    let before = construction.budget.work;
    construction.extract(leaf).unwrap();
    assert_eq!(
        before - construction.budget.work,
        1 + 64 + 1,
        "one node: its one-byte name, the 64-byte node overhead, and the node"
    );
}
#[test]
fn an_interned_name_or_literal_payload_remains_visible_to_tree_materialization() {
    for term in [
        Term::Global("x".repeat(65_536)),
        Term::Lit(crate::Literal::Text("t".repeat(65_536))),
    ] {
        let mut limits = budget();
        let mut construction = Construction::new(&mut limits).unwrap();
        let root = construction.intern(&term).unwrap();
        construction.budget.allocation = 32;
        assert!(
            matches!(
                construction.extract(root),
                Err(RecursorGenerationError::ResourceLimit)
            ),
            "{term:?}"
        );
    }
}
#[test]
fn an_interned_universe_is_charged_to_the_sort_or_constant_that_copies_it() {
    let big = || Universe::Var("u".repeat(65_536));
    for term in [
        Term::Sort(big()),
        Term::Const {
            name: "c".into(),
            levels: vec![Universe::Succ(Box::new(big()))],
        },
    ] {
        let mut limits = budget();
        let mut construction = Construction::new(&mut limits).unwrap();
        let root = construction.intern(&term).unwrap();
        construction.budget.allocation = 32;
        assert!(
            matches!(
                construction.extract(root),
                Err(RecursorGenerationError::ResourceLimit)
            ),
            "{term:?}"
        );
    }
}
