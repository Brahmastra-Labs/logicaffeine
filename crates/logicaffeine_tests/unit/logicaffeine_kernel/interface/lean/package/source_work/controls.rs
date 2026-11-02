use super::*;
#[test]
fn empty_source_limits_refuse_before_construction() {
    for which in 0..3 {
        let mut limits = EnvironmentLimits::default();
        match which {
            0 => limits.max_depth = 0,
            1 => limits.max_nodes = 0,
            _ => limits.max_tree_nodes = 0,
        }
        assert!(SourceWork::new(&limits).is_err());
    }
}
#[test]
fn source_policy_arithmetic_overflow_is_resource_refusal() {
    let mut limits = EnvironmentLimits::default();
    limits.max_tree_nodes = u64::MAX;
    let error = SourceWork::new(&limits)
        .err()
        .expect("overflow must refuse");
    assert_eq!(error.reason().code(), DiagnosticCode::ResourceLimit);
}
#[test]
fn source_work_calls_share_remaining_work() {
    let mut work = SourceWork::new(&EnvironmentLimits::default()).unwrap();
    let term = Term::Sort(crate::Universe::Type(0));
    let before = work.budget.work;
    assert!(work.levels(&[], &[&term]).unwrap());
    assert!(work.budget.work < before);
    work.budget.work = 0;
    let error = work
        .levels(&[], &[&term])
        .err()
        .expect("second operation must not replenish work");
    assert_eq!(error.reason().code(), DiagnosticCode::ResourceLimit);
    assert_eq!(work.budget.work, 0);
}

#[test]
fn raw_decode_allowance_is_shared_and_refuses_before_the_second_allocation() {
    let mut work = SourceWork::new(&EnvironmentLimits::default()).unwrap();
    work.budget.materialized_nodes = 3;
    work.reserve_source(2).unwrap();
    assert_eq!(work.budget.materialized_nodes, 1);
    let error = work
        .reserve_source(2)
        .err()
        .expect("raw source allocation cannot replenish");
    assert_eq!(error.reason().code(), DiagnosticCode::ResourceLimit);
    assert_eq!(work.budget.materialized_nodes, 1);
}
