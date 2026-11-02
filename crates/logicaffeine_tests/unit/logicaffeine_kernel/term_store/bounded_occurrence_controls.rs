use super::*;
fn v(n: &str) -> Term {
    Term::Var(n.into())
}
#[test]
fn bounded_count_uses_no_heap_backed_scratch_walk() {
    let body = Term::App(Box::new(v("x")), Box::new(v("x")));
    OCCURRENCE_HEAP_WALKS.with(|n| n.set(0));
    assert_eq!(
        bounded_occurrence_count_in_term(&body, "x", 64, 4096),
        Some(2)
    );
    assert_eq!(OCCURRENCE_HEAP_WALKS.with(|n| n.get()), 0);
}
#[test]
fn depth_and_node_allowances_have_exact_boundaries() {
    let body = Term::App(Box::new(v("x")), Box::new(v("y")));
    assert_eq!(bounded_occurrence_count_in_term(&body, "x", 2, 3), Some(1));
    assert_eq!(bounded_occurrence_count_in_term(&body, "x", 1, 3), None);
    assert_eq!(bounded_occurrence_count_in_term(&body, "x", 2, 2), None);
    assert_eq!(bounded_occurrence_count_in_term(&v("x"), "x", 0, 1), None);
    assert_eq!(bounded_occurrence_count_in_term(&v("x"), "x", 1, 0), None);
}
#[test]
fn wide_match_and_simultaneous_binders_match_the_general_counter() {
    let wide = Term::Match {
        discriminant: Box::new(v("x")),
        motive: Box::new(v("x")),
        cases: vec![v("x"); 512],
    };
    let shadow = Term::MutualFix {
        defs: vec![("x".into(), v("x")), ("y".into(), v("x"))],
        index: 1,
    };
    let binding = Term::Let {
        name: "x".into(),
        ty: Box::new(v("x")),
        value: Box::new(v("x")),
        body: Box::new(v("x")),
    };
    for term in [wide, shadow, binding] {
        assert_eq!(
            bounded_occurrence_count_in_term(&term, "x", 64, 4096),
            Some(occurrence_count_in_term(&term, "x"))
        );
    }
}
