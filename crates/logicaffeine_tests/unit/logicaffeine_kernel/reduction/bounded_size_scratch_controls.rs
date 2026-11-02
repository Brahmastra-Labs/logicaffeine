use super::*;
fn atom() -> Term {
    Term::Global("Nat".into())
}
fn reset() {
    BOUNDED_SIZE_PENDING_PEAK.with(|p| p.set(0));
}
fn peak() -> usize {
    BOUNDED_SIZE_PENDING_PEAK.with(std::cell::Cell::get)
}
fn cases(count: usize) -> Term {
    Term::Match {
        discriminant: Box::new(atom()),
        motive: Box::new(atom()),
        cases: vec![atom(); count],
    }
}
#[test]
fn exact_4096_nodes_remain_admitted_and_4097_are_refused() {
    reset();
    assert_eq!(bounded_conversion_size(&cases(4093)), Some(4096));
    assert!(peak() <= 4096);
    assert_eq!(bounded_conversion_size(&cases(4094)), None);
}
#[test]
fn wide_match_refuses_before_allocating_its_pending_children() {
    reset();
    assert_eq!(bounded_conversion_size(&cases(100_000)), None);
    assert!(peak() <= 4096, "actual pending scratch slots: {}", peak());
}
#[test]
fn wide_mutual_block_refuses_before_allocating_its_pending_bodies() {
    let term = Term::MutualFix {
        defs: (0..100_000).map(|i| (format!("f{i}"), atom())).collect(),
        index: 0,
    };
    reset();
    assert_eq!(bounded_conversion_size(&term), None);
    assert!(peak() <= 4096, "actual pending scratch slots: {}", peak());
}
