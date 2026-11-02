//! Bounds checks survive calls that resize a statement's destination.
use logicaffeine_compile::compile::{tw_outcome, vm_outcome, vm_outcome_tiered};
use logicaffeine_compile::optimization::Tier;

fn refuses_after_shrinking(source: &str) {
    let reference = tw_outcome(source);
    let error = reference.error.as_deref().expect("the selected item no longer exists");
    assert!(error.to_ascii_lowercase().contains("bound"), "{reference:?}");
    let plain = vm_outcome(source);
    assert_eq!(plain.error, reference.error, "VM: {source}");
    assert_eq!(plain.output.lines().collect::<Vec<_>>(), reference.output.lines().collect::<Vec<_>>());
    let optimized = vm_outcome_tiered(source, &[], Tier::T3, None);
    assert_eq!(optimized.error, reference.error, "optimized VM: {source}");
    assert_eq!(optimized.output.lines().collect::<Vec<_>>(), reference.output.lines().collect::<Vec<_>>());
}

#[test]
fn a_statement_value_can_shrink_its_destination_before_the_write() {
    refuses_after_shrinking("## To shrink (values: mutable Seq of Int) -> Int:\n    Set values to [9].\n    Return 7.\n## Main\nLet mutable values be [1, 2].\nSet item 2 of values to shrink(values).\n");
}

#[test]
fn a_loop_value_can_shrink_its_destination_before_the_write() {
    refuses_after_shrinking("## To shrink (values: mutable Seq of Int) -> Int:\n    Set values to [9].\n    Return 7.\n## Main\nLet mutable values be [1, 2].\nRepeat for i from 1 to 2:\n    Set item i of values to shrink(values).\n");
}

#[test]
fn a_global_replacement_invalidates_the_destination_bounds() {
    refuses_after_shrinking("## To shrink () -> Int:\n    Set values to [9].\n    Return 7.\n## Main\nLet mutable values be [1, 2].\nSet item 2 of values to shrink().\n");
}
