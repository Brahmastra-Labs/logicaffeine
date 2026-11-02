use super::*;
use crate::Universe;
fn sort(n: u32) -> Term {
    Term::Sort(Universe::Type(n))
}
fn wide_type() -> Term {
    let mut term = sort(0);
    for _ in 0..12 {
        term = Term::Pi {
            param: "_".into(),
            param_type: Box::new(term.clone()),
            body_type: Box::new(term),
        };
    }
    term
}
fn at(name: &str) -> Term {
    Term::Const {
        name: name.into(),
        levels: vec![Universe::Type(0)],
    }
}
#[test]
fn oversized_polymorphic_body_is_declined_before_cloning() {
    let mut c = Context::new();
    c.declare_universe_poly("wide_body", vec!["u".into()], sort(1), wide_type())
        .unwrap();
    CONST_STEP_CLONES.with(|n| n.set(0));
    let result = bounded_delta_head(&c, &at("wide_body"));
    let copies = CONST_STEP_CLONES.with(|n| n.get());
    assert!(result.is_none());
    assert_eq!(
        copies, 0,
        "an oversized candidate must be preflighted while borrowed"
    );
}
#[test]
fn oversized_polymorphic_header_is_declined_before_cloning() {
    let mut c = Context::new();
    let ty = wide_type();
    c.add_declaration("wide_assumption".into(), ty.clone());
    c.declare_universe_poly(
        "wide_header",
        vec!["u".into()],
        ty,
        Term::Global("wide_assumption".into()),
    )
    .unwrap();
    CONST_STEP_CLONES.with(|n| n.set(0));
    let result = bounded_delta_head(&c, &at("wide_header"));
    let copies = CONST_STEP_CLONES.with(|n| n.get());
    assert!(
        result.is_none(),
        "bounded probe must account for its header-copy work too"
    );
    assert_eq!(copies, 0);
}
#[test]
fn small_explicit_universe_delta_still_uses_the_checked_resolver() {
    let mut c = Context::new();
    let u = Universe::Var("u".into());
    c.declare_universe_poly(
        "small",
        vec!["u".into()],
        Term::Sort(u.clone().succ()),
        Term::Sort(u),
    )
    .unwrap();
    CONST_STEP_CLONES.with(|n| n.set(0));
    let (_, body) = bounded_delta_head(&c, &at("small")).unwrap();
    assert_eq!(body, sort(0));
    assert_eq!(CONST_STEP_CLONES.with(|n| n.get()), 2);
}
