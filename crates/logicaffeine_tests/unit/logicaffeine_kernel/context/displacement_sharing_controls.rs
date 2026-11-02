use super::*;
use std::collections::BTreeSet;

fn prop() -> Term {
    Term::Sort(Universe::Prop)
}
fn g(name: &str) -> Term {
    Term::Global(name.into())
}
fn checked_context() -> Context {
    let mut c = Context::new();
    c.set_auto_auxiliaries(false);
    c.add_inductive("P", prop());
    c.add_constructor("p", "P", g("P"));
    c.with_equations_suspended(|c| c.declare("body", g("P"), g("p")))
        .unwrap();
    c.declare_opaque_checked("proof", vec![], g("P"), g("body"))
        .unwrap();
    c
}
macro_rules! shared {
    ($left:expr, $right:expr; $($field:ident),+ $(,)?) => { $(
        assert!(Arc::ptr_eq(&$left.$field, &$right.$field), stringify!($field));
    )+ };
}

#[test]
fn absent_displacement_preserves_every_unaffected_registry_allocation() {
    let mut c = checked_context();
    let snapshot = c.clone();
    c.displace("never_registered");
    shared!(c, snapshot; derived_aux, derived_aux_owner, constructor_order, constructors,
        inductives, poly_types, binder_kinds, implicit_args, param_defaults, universe_polys,
        pending, inductive_params, inductive_inferred_params, definitions, definition_deps,
        definition_heights, declarations, structures, mutual_blocks, eliminations, mentions,
        mentioned_by, checked_opaque, checked_opaque_graph, checked_opaque_dependents,
        quotient_primitives, referenced, pending_waiters);
    assert_eq!(c.get_definition_body("body"), Some(&g("p")));
    assert!(c.checked_opaque_dependencies("proof").is_some());
}

#[test]
fn fresh_public_declaration_forks_its_registry_but_not_unrelated_payload_maps() {
    let mut c = checked_context();
    let snapshot = c.clone();
    c.add_declaration("fresh", prop());
    assert_eq!(c.get_global("fresh"), Some(&prop()));
    assert!(snapshot.get_global("fresh").is_none());
    assert!(!Arc::ptr_eq(&c.declarations, &snapshot.declarations));
    shared!(c, snapshot; definitions, universe_polys, constructors, constructor_order,
        inductives, poly_types, structures, mutual_blocks, checked_opaque,
        checked_opaque_graph, checked_opaque_dependents, quotient_primitives);
    assert!(c.checked_opaque_dependencies("proof").is_some());
}

#[test]
fn absent_displacement_preserves_weak_arithmetic_generation_but_real_change_invalidates() {
    let mut c = Context::new();
    let count = std::cell::Cell::new(0);
    let verify = || {
        count.set(count.get() + 1);
        true
    };
    assert!(c.cached_integer_arithmetic("add", verify));
    let generation = ArithmeticGeneration::of(&c);
    c.displace("missing");
    assert!(generation.matches(&c));
    assert!(c.cached_integer_arithmetic("add", verify));
    assert_eq!(count.get(), 1);
    c.add_declaration("fresh", prop());
    assert!(!generation.matches(&c));
    assert!(c.cached_integer_arithmetic("add", verify));
    assert_eq!(count.get(), 2);
}

#[test]
fn existing_definition_displacement_removes_payload_and_keeps_snapshot_and_cycle_summary() {
    let mut c = checked_context();
    let snapshot = c.clone();
    c.displace("body");
    assert!(c.get_definition_body("body").is_none());
    assert!(!c.definition_deps.contains_key("body"));
    assert!(!c.definition_heights.contains_key("body"));
    assert!(c.checked_opaque_dependencies("proof").is_none());
    assert_eq!(snapshot.get_definition_body("body"), Some(&g("p")));
    assert!(snapshot.checked_opaque_dependencies("proof").is_some());
    assert!(!Arc::ptr_eq(&c.definitions, &snapshot.definitions));
    shared!(c, snapshot; referenced, constructors, constructor_order, inductives);
}

#[test]
fn constructor_and_owner_displacement_keep_order_and_other_family_consistent() {
    let mut c = Context::new();
    c.set_auto_auxiliaries(false);
    c.add_inductive("A", prop());
    c.add_constructor("a", "A", g("A"));
    c.add_constructor("a2", "A", g("A"));
    c.add_inductive("B", prop());
    c.add_constructor("b", "B", g("B"));
    let snapshot = c.clone();
    c.displace("a");
    assert_eq!(
        c.constructor_order.get("A").unwrap(),
        &vec!["a2".to_string()]
    );
    assert!(!c.constructors.contains_key("a"));
    assert!(snapshot.constructors.contains_key("a"));
    c.displace("A");
    assert!(!c.constructor_order.contains_key("A"));
    assert!(!c.constructors.contains_key("a2"));
    assert!(c.constructors.contains_key("b"));
    assert_eq!(
        c.constructor_order.get("B").unwrap(),
        &vec!["b".to_string()]
    );
}

#[test]
fn recursive_auxiliary_cleanup_preserves_a_disowned_user_replacement() {
    let mut c = Context::new();
    for name in ["owner", "child", "grandchild", "replacement"] {
        c.add_declaration(name, prop());
    }
    c.record_derived_aux("owner", "child");
    c.record_derived_aux("child", "grandchild");
    c.record_derived_aux("owner", "replacement");
    let snapshot = c.clone();
    c.displace("replacement");
    c.add_declaration("replacement", prop());
    assert!(!c.derived_aux_owner.contains_key("replacement"));
    c.displace("owner");
    for name in ["owner", "child", "grandchild"] {
        assert!(c.get_global(name).is_none());
    }
    assert_eq!(c.get_global("replacement"), Some(&prop()));
    for name in ["owner", "child", "grandchild", "replacement"] {
        assert!(snapshot.get_global(name).is_some());
    }
}

#[test]
fn outgoing_mentions_are_removed_but_incoming_obligations_and_monotone_edges_remain() {
    let mut c = Context::new();
    c.record_mention_edges("victim", ["dependency".to_string()]);
    c.record_mention_edges("user", ["victim".to_string()]);
    Arc::make_mut(&mut c.eliminations).insert("victim".into(), Eliminations::default());
    Arc::make_mut(&mut c.pending).insert("victim".into(), "dependency".into());
    Arc::make_mut(&mut c.pending).insert("user".into(), "victim".into());
    Arc::make_mut(&mut c.referenced).insert("victim".into());
    let snapshot = c.clone();
    c.displace("victim");
    assert!(!c.mentions.contains_key("victim"));
    assert!(!c.eliminations.contains_key("victim"));
    assert!(!c.mentioned_by.contains_key("dependency"));
    assert_eq!(
        c.mentioned_by.get("victim"),
        Some(&BTreeSet::from(["user".to_string()]))
    );
    assert!(!c.pending.contains_key("victim"));
    assert_eq!(c.pending.get("user").map(String::as_str), Some("victim"));
    shared!(c, snapshot; referenced, pending_waiters);
    assert!(snapshot.pending.contains_key("victim"));
}

#[test]
fn checked_opaque_dependents_are_invalidated_transitively_in_only_the_mutated_context() {
    let mut c = checked_context();
    c.declare_opaque_checked("second", vec![], g("P"), g("proof"))
        .unwrap();
    c.declare_opaque_checked("independent", vec![], g("P"), g("p"))
        .unwrap();
    let snapshot = c.clone();
    c.displace("body");
    assert!(c.checked_opaque_dependencies("proof").is_none());
    assert!(c.checked_opaque_dependencies("second").is_none());
    assert!(c.checked_opaque_dependencies("independent").is_some());
    assert!(snapshot.checked_opaque_dependencies("proof").is_some());
    assert!(snapshot.checked_opaque_dependencies("second").is_some());
    assert!(!c.checked_opaque_graph.contains_key("body"));
    assert!(!c.checked_opaque_dependents.contains_key("body"));
}

#[test]
fn absent_global_name_still_invalidates_quotient_and_opaque_dependency_edges() {
    let mut c = Context::new();
    c.record_checked_opaque_edges("dependent".into(), BTreeSet::from(["missing".into()]));
    for (name, dependency) in [("affected", "missing"), ("unaffected", "other")] {
        Arc::make_mut(&mut c.quotient_primitives).insert(
            name.into(),
            crate::quotient::Certificate {
                kind: crate::quotient::Kind::Type,
                dependencies: vec![dependency.into()],
                equality: None,
            },
        );
    }
    assert!(c.get_global("missing").is_none());
    let snapshot = c.clone();
    c.displace("missing");
    assert!(!c.checked_opaque_graph.contains_key("dependent"));
    assert!(!c.checked_opaque_dependents.contains_key("missing"));
    assert!(!c.quotient_primitives.contains_key("affected"));
    assert!(c.quotient_primitives.contains_key("unaffected"));
    assert!(snapshot.quotient_primitives.contains_key("affected"));
    shared!(c, snapshot; declarations, definitions);
}

#[test]
fn unrelated_quotient_certificate_remains_shared_on_absent_dependency() {
    let mut c = Context::new();
    Arc::make_mut(&mut c.quotient_primitives).insert(
        "q".into(),
        crate::quotient::Certificate {
            kind: crate::quotient::Kind::Type,
            dependencies: vec!["other".into()],
            equality: None,
        },
    );
    let snapshot = c.clone();
    c.displace("missing");
    shared!(c, snapshot; quotient_primitives);
    assert_eq!(
        c.quotient_primitives["q"].dependencies,
        vec!["other".to_string()]
    );
}
