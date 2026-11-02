use super::*;
fn sort(n: &str) -> Term {
    Term::Sort(Universe::Var(n.into()))
}
#[test]
fn declared_success_does_not_collect_an_owned_level_telescope() {
    let ty = Term::App(Box::new(sort("u")), Box::new(sort("v")));
    LEVEL_COLLECTION_CALLS.with(|n| n.set(0));
    let map = resolve_const_levels(
        "value",
        Some(&["v".into(), "u".into()]),
        &ty,
        &[Universe::Prop, Universe::Type(2)],
    )
    .unwrap();
    assert_eq!(map.get("v"), Some(&Universe::Prop));
    assert_eq!(map.get("u"), Some(&Universe::Type(2)));
    assert_eq!(LEVEL_COLLECTION_CALLS.with(|n| n.get()), 0);
}
#[test]
fn first_unbound_level_and_closure_before_arity_are_unchanged() {
    let ty = Term::App(
        Box::new(Term::Const {
            name: "nested".into(),
            levels: vec![Universe::Max(
                Box::new(Universe::Var("first".into())),
                Box::new(Universe::Var("second".into())),
            )],
        }),
        Box::new(sort("third")),
    );
    let result = resolve_const_levels("bad", Some(&["bound".into()]), &ty, &[]);
    assert!(
        matches!(result,Err(ConstError::LevelNotClosed{name,level}) if name=="bad" && level=="first")
    );
}
#[test]
fn inferred_telescope_is_collected_once_in_first_occurrence_order() {
    let ty = Term::App(
        Box::new(sort("v")),
        Box::new(Term::App(Box::new(sort("u")), Box::new(sort("v")))),
    );
    LEVEL_COLLECTION_CALLS.with(|n| n.set(0));
    let map =
        resolve_const_levels("inferred", None, &ty, &[Universe::Prop, Universe::Type(3)]).unwrap();
    assert_eq!(map.get("v"), Some(&Universe::Prop));
    assert_eq!(map.get("u"), Some(&Universe::Type(3)));
    assert_eq!(LEVEL_COLLECTION_CALLS.with(|n| n.get()), 1);
}
#[test]
fn duplicate_declared_positions_keep_the_last_mapping_and_original_arity() {
    let params = vec!["u".into(), "u".into()];
    let map = resolve_const_levels(
        "duplicate",
        Some(&params),
        &sort("u"),
        &[Universe::Prop, Universe::Type(4)],
    )
    .unwrap();
    assert_eq!(map.len(), 1);
    assert_eq!(map.get("u"), Some(&Universe::Type(4)));
    assert!(matches!(
        resolve_const_levels("duplicate", Some(&params), &sort("u"), &[Universe::Prop]),
        Err(ConstError::Arity {
            expected: 2,
            got: 1,
            ..
        })
    ));
}
#[test]
fn empty_telescope_and_simultaneous_symbolic_levels_preserve_the_map() {
    assert!(
        resolve_const_levels("mono", Some(&[]), &Term::Sort(Universe::Type(0)), &[])
            .unwrap()
            .is_empty()
    );
    let ty = Term::App(Box::new(sort("u")), Box::new(sort("v")));
    let map = resolve_const_levels(
        "simultaneous",
        Some(&["u".into(), "v".into()]),
        &ty,
        &[Universe::Var("v".into()), Universe::Type(2)],
    )
    .unwrap();
    assert_eq!(map.get("u"), Some(&Universe::Var("v".into())));
}
