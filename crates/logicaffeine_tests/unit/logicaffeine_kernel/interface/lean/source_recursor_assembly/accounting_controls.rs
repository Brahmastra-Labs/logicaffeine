use super::*;
#[test]
fn projection_level_copy_refuses_before_unadmitted_materialization() {
    let mut budget = RecursorBudget {
        max_depth: crate::type_checker::MAX_TERM_DEPTH,
        max_tree_nodes: 1_000,
        max_store_nodes: 1_000,
        work: 1_000_000,
        allocation: 1_000_000,
        materialized_nodes: 0,
    };
    let mut d = Construction::new(&mut budget).unwrap();
    let level = Universe::Max(
        Box::new(Universe::Var("u".into())),
        Box::new(Universe::Type(1)),
    );
    let error = copy_projection_level(&mut d, &level).unwrap_err();
    assert_eq!(error.reason().code(), DiagnosticCode::ResourceLimit);
}
#[test]
fn projection_level_copy_preserves_universe_syntax() {
    let mut budget = RecursorBudget {
        max_depth: crate::type_checker::MAX_TERM_DEPTH,
        max_tree_nodes: 1_000,
        max_store_nodes: 1_000,
        work: 1_000_000,
        allocation: 1_000_000,
        materialized_nodes: 1_000,
    };
    let mut d = Construction::new(&mut budget).unwrap();
    let level = Universe::IMax(
        Box::new(Universe::Var("u".into())),
        Box::new(Universe::Type(1)),
    );
    assert_eq!(copy_projection_level(&mut d, &level).unwrap(), level);
}
