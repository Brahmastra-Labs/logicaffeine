use super::*;
fn budget() -> RecursorBudget {
    RecursorBudget {
        max_depth: 64,
        max_tree_nodes: 100_000,
        max_store_nodes: 100_000,
        work: 100_000_000,
        allocation: 100_000_000,
        materialized_nodes: 100_000,
    }
}
#[test]
fn simultaneous_replacements_and_universe_simplification_match_existing_operation() {
    let u = Universe::Var("u".into());
    let v = Universe::Var("v".into());
    let term = Term::Const {
        name: "poly".into(),
        levels: vec![
            u.clone(),
            Universe::Succ(Box::new(u.clone())),
            Universe::Max(Box::new(u.clone()), Box::new(v.clone())),
            Universe::IMax(Box::new(v), Box::new(u)),
        ],
    };
    let substitutions = [
        HashMap::from([
            ("u".into(), Universe::Var("v".into())),
            ("v".into(), Universe::Prop),
        ]),
        HashMap::from([
            ("u".into(), Universe::Type(u32::MAX)),
            ("v".into(), Universe::SProp),
        ]),
    ];
    for substitution in substitutions {
        assert_eq!(
            instantiate_universes_bounded(&term, &substitution, &mut budget()).unwrap(),
            crate::instantiate_universes(&term, &substitution)
        );
    }
}
#[test]
fn replacement_depth_is_admitted_before_materialization() {
    let term = Term::Sort(Universe::Var("u".into()));
    let replacement = Universe::Succ(Box::new(Universe::Succ(Box::new(Universe::Var(
        "v".into(),
    )))));
    let mut budget = budget();
    budget.max_depth = 2;
    let before = budget.materialized_nodes;
    assert!(matches!(
        instantiate_universes_bounded(
            &term,
            &HashMap::from([("u".into(), replacement)]),
            &mut budget
        ),
        Err(RecursorGenerationError::ResourceLimit)
    ));
    assert_eq!(budget.materialized_nodes, before);
}
#[test]
fn repeated_replacements_exhaust_one_output_allowance() {
    let term = Term::Const {
        name: "poly".into(),
        levels: vec![Universe::Var("u".into()); 8],
    };
    let replacement = Universe::Max(
        Box::new(Universe::Var("a".into())),
        Box::new(Universe::Var("b".into())),
    );
    let mut budget = budget();
    budget.max_tree_nodes = 24;
    assert!(matches!(
        instantiate_universes_bounded(
            &term,
            &HashMap::from([("u".into(), replacement)]),
            &mut budget
        ),
        Err(RecursorGenerationError::ResourceLimit)
    ));
}
#[test]
fn payload_copies_and_repeated_calls_share_allocation_limits() {
    let term = Term::Lit(Literal::Text("x".repeat(4096)));
    let mut budget = budget();
    budget.allocation = 300;
    instantiate_universes_bounded(&term, &HashMap::new(), &mut budget).unwrap();
    assert!(matches!(
        instantiate_universes_bounded(&term, &HashMap::new(), &mut budget),
        Err(RecursorGenerationError::ResourceLimit)
    ));
}
#[test]
fn zero_work_or_allocation_yields_no_constructed_tree() {
    for allocation in [true, false] {
        let mut budget = budget();
        if allocation {
            budget.allocation = 0;
        } else {
            budget.work = 0;
        }
        let before = budget.materialized_nodes;
        assert!(matches!(
            instantiate_universes_bounded(&Term::Hole, &HashMap::new(), &mut budget),
            Err(RecursorGenerationError::ResourceLimit)
        ));
        assert_eq!(budget.materialized_nodes, before);
    }
}
#[test]
fn replaced_source_names_are_charged_again_for_the_actual_lookup() {
    let name = "u".repeat(1024);
    let term = Term::Sort(Universe::Var(name.clone()));
    let substitution = HashMap::from([(name, Universe::Prop)]);
    let mut budget = budget();
    budget.work = 1500;
    let before = budget.materialized_nodes;
    assert!(matches!(
        instantiate_universes_bounded(&term, &substitution, &mut budget),
        Err(RecursorGenerationError::ResourceLimit)
    ));
    assert_eq!(budget.materialized_nodes, before);
}
