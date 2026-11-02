use super::*;
fn budget() -> RecursorBudget {
    RecursorBudget {
        max_depth: crate::type_checker::MAX_TERM_DEPTH,
        max_tree_nodes: 2_000_000,
        max_store_nodes: 2_000_000,
        work: 1_000_000_000_000,
        allocation: 1_000_000_000,
        materialized_nodes: 20_000_000,
    }
}
fn compare(ctx: &Context, name: &str) {
    let original = super::super::derive_recursor_universe_poly(ctx, name).unwrap();
    let bounded = derive_recursor_universe_poly_bounded(ctx, name, &mut budget()).unwrap();
    assert_eq!(original.0, bounded.0);
    assert!(crate::defeq_for_test(ctx, &original.1, &bounded.1));
    assert!(crate::defeq_for_test(ctx, &original.2, &bounded.2));
    assert!(crate::recheck(ctx, &bounded.2).is_ok());
}
#[test]
fn bounded_single_recursor_preserves_nat_computation() {
    let mut ctx = Context::new();
    crate::prelude::StandardLibrary::register(&mut ctx);
    compare(&ctx, "Nat");
}
#[test]
fn bounded_mutual_recursor_preserves_each_selected_component() {
    let mut ctx = Context::new();
    let block = ["Left", "Right"].map(|name| crate::MutualInductive {
        name: name.into(),
        sort: Term::Sort(Universe::Type(0)),
        num_params: 0,
        constructors: vec![(format!("make{name}"), Term::Global(name.into()))],
    });
    ctx.add_mutual_inductives(&block).unwrap();
    compare(&ctx, "Left");
    compare(&ctx, "Right");
}
#[test]
fn shallow_constructor_inventory_cannot_allocate_an_over_depth_recursor_spine() {
    let mut ctx = Context::new();
    ctx.add_inductive("Wide", Term::Sort(Universe::Type(0)));
    for i in 0..24 {
        ctx.add_constructor(&format!("wide{i}"), "Wide", Term::Global("Wide".into()));
    }
    let mut limits = budget();
    limits.max_depth = 12;
    let result = derive_recursor_universe_poly_bounded(&ctx, "Wide", &mut limits);
    assert!(
        matches!(result, Err(RecursorGenerationError::ResourceLimit)),
        "the shallow declaration inventory generates a deeper recursor"
    );
}
#[test]
fn shared_dag_expansion_refuses_before_tree_materialization() {
    let mut limits = budget();
    limits.max_tree_nodes = 32;
    let before = limits.materialized_nodes;
    let mut d = Dag {
        store: TermStore::new(),
        budget: &mut limits,
        payload_envelope: 0,
        input_envelope: 0,
    };
    let mut t = d.var("x").unwrap();
    for _ in 0..4 {
        t = d.app(t, t).unwrap();
    }
    assert_eq!(d.store.tree_size(t), 31);
    assert!(d.app(t, t).is_err());
    assert_eq!(
        d.budget.materialized_nodes, before,
        "no owned tree was extracted"
    );
}

#[cfg(feature = "lean")]
#[test]
fn bounded_authentic_polymorphic_mutual_recursors_preserve_universe_telescopes() {
    let export =
        include_str!("../../../../tests/fixtures/lean_arena/ordinary-mutual-polymorphic.ndjson");
    let (ctx, _) = crate::interface::lean::import(export).unwrap();
    compare(&ctx, "/s:\"ArenaTree\"");
    compare(&ctx, "/s:\"ArenaForest\"");
}
