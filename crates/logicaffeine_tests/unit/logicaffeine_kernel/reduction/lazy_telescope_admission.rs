use super::*;
use crate::Universe;
fn sort(n: u32) -> Term {
    Term::Sort(Universe::Type(n))
}
fn context(params: Vec<String>) -> Context {
    let mut c = Context::new();
    // Checked definitions may legitimately have unused explicit parameters.
    c.declare_universe_poly("small", params, sort(1), sort(0))
        .unwrap();
    c
}
fn at(count: usize) -> Term {
    Term::Const {
        name: "small".into(),
        levels: vec![Universe::Type(0); count],
    }
}
fn probe(c: &Context, t: &Term) -> Option<(usize, Term)> {
    bounded_delta_head_with(
        c,
        t,
        Transparency::Excluding(&|_| false),
        bounded_conversion_size,
    )
}
fn reset() {
    CONST_STEP_CLONES.with(|n| n.set(0));
}
fn copies() -> usize {
    CONST_STEP_CLONES.with(|n| n.get())
}
#[test]
fn oversized_unused_parameter_name_is_refused_before_const_step_copies() {
    let c = context(vec!["u".repeat(257)]);
    reset();
    assert!(probe(&c, &at(1)).is_none());
    assert_eq!(copies(), 0);
}
#[test]
fn oversized_unused_telescope_is_refused_before_arity_resolution_copies() {
    let c = context((0..17).map(|i| format!("u{i}")).collect());
    reset();
    assert!(probe(&c, &at(1)).is_none());
    assert_eq!(
        copies(),
        0,
        "a tiny reference must not clone stored parameters before rejecting arity"
    );
}
#[test]
fn exact_telescope_and_name_boundaries_still_unfold() {
    let c = context(
        (0..16)
            .map(|i| format!("{i:02}{}", "u".repeat(254)))
            .collect(),
    );
    reset();
    assert_eq!(probe(&c, &at(16)).unwrap().1, sort(0));
    assert_eq!(copies(), 2);
}
#[test]
fn recursive_application_head_keeps_telescope_admission() {
    let c = context(vec!["u".repeat(257)]);
    let term = Term::App(Box::new(at(1)), Box::new(sort(0)));
    reset();
    assert!(probe(&c, &term).is_none());
    assert_eq!(copies(), 0);
}
#[test]
fn original_all_wrapper_keeps_legal_large_telescope_behavior() {
    let c = context(
        (0..17)
            .map(|i| format!("{i:02}{}", "u".repeat(257)))
            .collect(),
    );
    reset();
    assert_eq!(bounded_delta_head(&c, &at(17)).unwrap().1, sort(0));
    assert_eq!(copies(), 2);
}
