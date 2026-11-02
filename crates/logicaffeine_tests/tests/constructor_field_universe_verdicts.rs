//! Semantic constructor universe failures remain distinct from incomplete checking.
use logicaffeine_kernel::interface::lean as arena;
use logicaffeine_kernel::{Context, KernelError, Term, Universe};
fn sort(u: Universe) -> Term {
    Term::Sort(u)
}
fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn arrow(a: Term, b: Term) -> Term {
    Term::Pi {
        param: "field".into(),
        param_type: Box::new(a),
        body_type: Box::new(b),
    }
}
fn check(target: Universe, field: Term) -> Result<(), KernelError> {
    let mut c = Context::new();
    c.add_inductive("Box", sort(target));
    c.add_constructor_checked("mk", "Box", arrow(field, g("Box")))
}
#[test]
fn official_excessive_constructor_field_universe_is_rejected() {
    let result = arena::check_str(include_str!(
        "fixtures/lean_arena/061_typeWithTooHighTypeField.mk.ndjson"
    ));
    assert!(
        matches!(result, Err(arena::Verdict::Reject(_))),
        "{result:?}"
    );
}
#[test]
fn concrete_excessive_field_is_a_semantic_declaration_error() {
    let result = check(Universe::Type(0), sort(Universe::Type(0)));
    assert!(
        matches!(result, Err(KernelError::InvalidDeclaration(_))),
        "{result:?}"
    );
}
#[test]
fn sufficiently_large_target_accepts_the_same_field() {
    check(Universe::Type(1), sort(Universe::Type(0))).unwrap();
}
#[test]
fn semantically_prop_target_keeps_its_impredicative_field_exemption() {
    // Deliberately retain the raw expression rather than calling the simplifying imax builder.
    let target = Universe::IMax(Box::new(Universe::Type(9)), Box::new(Universe::Prop));
    check(target, sort(Universe::Type(12))).unwrap();
}
fn symbolic(target: Universe) -> Result<(), KernelError> {
    let mut c = Context::new();
    let u = Universe::Var("u".into());
    c.add_universe_poly_inductive("Box", vec!["u".into()], sort(target), 0);
    let carrier = Term::Const {
        name: "Box".into(),
        levels: vec![u.clone()],
    };
    c.add_constructor_checked("mk", "Box", arrow(sort(u), carrier))
}
#[test]
fn symbolic_excessive_field_is_rejected_for_all_instantiations_required_by_the_declaration() {
    let result = symbolic(Universe::Var("u".into()));
    assert!(
        matches!(result, Err(KernelError::InvalidDeclaration(_))),
        "{result:?}"
    );
}
#[test]
fn symbolic_matching_field_universe_accepts() {
    symbolic(Universe::Var("u".into()).succ()).unwrap();
}
#[test]
fn unfinished_field_inference_is_not_reclassified_as_a_universe_mismatch() {
    let result = check(Universe::Type(0), Term::Hole);
    assert!(
        matches!(result, Err(KernelError::CannotInferHole)),
        "{result:?}"
    );
}
