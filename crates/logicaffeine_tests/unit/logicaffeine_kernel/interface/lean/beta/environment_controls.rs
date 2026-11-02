//! Resource and scoping controls for the closed-type source environment.
//!
//! Intentionally unwired until the parent schedules the RED run. These tests
//! specify observable lookup/typing behavior, not a run-record representation.
use super::*;
use serde_json::json;

fn empty_pass() -> Certificates {
    Certificates::new(Context::new(), &HashSet::new(), &[], &mut 10_000).unwrap()
}
fn checked_type(pass: &mut Certificates, level: u32) -> TermId {
    pass.type_id(&Term::Sort(Universe::Type(level)))
        .unwrap()
        .expect("closed independently checked universe type")
}
fn application(index_value: usize) -> Importer<'static> {
    let mut importer = Importer::new(false);
    importer.levels.insert(1, Universe::Type(0));
    importer.level_vars.insert(1, HashSet::new());
    importer.exprs.insert(0, json!({"ie":0,"sort":1}));
    importer.exprs.insert(1, json!({"ie":1,"bvar":0}));
    importer.exprs.insert(
        2,
        json!({"ie":2,"lam":{
        "name":0,"type":0,"body":1,"binderInfo":"default"}}),
    );
    importer.exprs.insert(3, json!({"ie":3,"bvar":index_value}));
    importer
        .exprs
        .insert(4, json!({"ie":4,"app":{"fn":2,"arg":3}}));
    importer
}

#[test]
fn four_thousand_uniform_binders_and_every_index_fit_twenty_thousand_steps() {
    // Reserve at most one fifth of the unchanged 100k source allowance for
    // environments: source traversal and checked rule witnesses still need room.
    let mut pass = empty_pass();
    let ty = checked_type(&mut pass, 0);
    let mut env = 0;
    let mut allowance = 20_000;
    for depth in 1..=4000 {
        env = pass
            .extend(env, ty, &mut allowance)
            .unwrap_or_else(|e| panic!("uniform extension at depth {depth}: {e:?}"));
    }
    for index in 0..4000 {
        assert_eq!(
            pass.lookup(env, index, &mut allowance).unwrap(),
            Some(ty),
            "de Bruijn index {index} must retain its own admitted binding"
        );
    }
    assert!(
        allowance > 0,
        "the environment must leave allowance for its caller"
    );
}

#[test]
fn mixed_runs_preserve_types_at_every_transition() {
    let mut pass = empty_pass();
    let types = [
        checked_type(&mut pass, 0),
        checked_type(&mut pass, 1),
        checked_type(&mut pass, 2),
    ];
    let mut env = 0;
    let mut bindings = Vec::new();
    let mut allowance = 20_000;
    for (which, length) in [(0, 63), (1, 1), (0, 64), (2, 2), (1, 127), (2, 1), (0, 128)] {
        for _ in 0..length {
            env = pass.extend(env, types[which], &mut allowance).unwrap();
            bindings.push(types[which]);
        }
    }
    for (index, ty) in bindings.iter().rev().enumerate() {
        assert_eq!(
            pass.lookup(env, index, &mut allowance).unwrap(),
            Some(*ty),
            "wrong type at mixed-run index {index}"
        );
    }
    assert_eq!(
        pass.lookup(env, bindings.len(), &mut allowance).unwrap(),
        None
    );
}

#[test]
fn alternating_types_keep_far_lookup_bounded() {
    // Compression must not turn the pre-existing logarithmic general case
    // into a linear walk over one-element runs.
    let mut pass = empty_pass();
    let types = [checked_type(&mut pass, 0), checked_type(&mut pass, 1)];
    let mut env = 0;
    let mut construction_allowance = 100_000;
    for index in 0..4096 {
        env = pass
            .extend(env, types[index % 2], &mut construction_allowance)
            .unwrap();
    }
    let mut lookup_allowance = 64;
    assert_eq!(
        pass.lookup(env, 4095, &mut lookup_allowance).unwrap(),
        Some(types[0])
    );
    assert_eq!(pass.lookup(env, 4096, &mut lookup_allowance).unwrap(), None);
}

#[test]
fn extending_a_branch_does_not_change_its_saved_parent_or_sibling() {
    let mut pass = empty_pass();
    let a = checked_type(&mut pass, 0);
    let b = checked_type(&mut pass, 1);
    let parent = pass.extend(0, a, &mut 1000).unwrap();
    let left = pass.extend(parent, a, &mut 1000).unwrap();
    let right = pass.extend(parent, b, &mut 1000).unwrap();
    let deeper = pass.extend(left, b, &mut 1000).unwrap();
    for (env, expected) in [
        (parent, vec![a]),
        (left, vec![a, a]),
        (right, vec![b, a]),
        (deeper, vec![b, a, a]),
    ] {
        for (index, ty) in expected.iter().enumerate() {
            assert_eq!(pass.lookup(env, index, &mut 1000).unwrap(), Some(*ty));
        }
        assert_eq!(pass.lookup(env, expected.len(), &mut 1000).unwrap(), None);
    }
}

#[test]
fn far_and_out_of_bounds_uniform_indices_do_not_wrap() {
    let mut pass = empty_pass();
    let ty = checked_type(&mut pass, 0);
    let mut env = 0;
    let mut allowance = 24_000;
    for _ in 0..4096 {
        env = pass.extend(env, ty, &mut allowance).unwrap();
    }
    assert_eq!(pass.lookup(env, 4095, &mut 100).unwrap(), Some(ty));
    assert_eq!(pass.lookup(env, 4096, &mut 100).unwrap(), None);
    assert_eq!(pass.lookup(env, usize::MAX, &mut 100).unwrap(), None);
    assert_eq!(pass.lookup(0, 0, &mut 100).unwrap(), None);
}

#[test]
fn exhausted_environment_operations_cannot_refill_the_allowance() {
    let mut pass = empty_pass();
    let ty = checked_type(&mut pass, 0);
    let env = pass.extend(0, ty, &mut 100).unwrap();
    let mut empty = 0;
    assert!(matches!(
        pass.extend(env, ty, &mut empty),
        Err(Verdict::Decline(_))
    ));
    assert_eq!(empty, 0);
    assert!(matches!(
        pass.lookup(env, 0, &mut empty),
        Err(Verdict::Decline(_))
    ));
    assert_eq!(empty, 0);
    // A refused operation does not destroy an existing immutable environment.
    assert_eq!(pass.lookup(env, 0, &mut 100).unwrap(), Some(ty));
}

#[test]
fn uniform_source_typing_checks_an_actual_outer_argument() {
    let importer = application(3999);
    let plan = dag::Plan::new(&importer, 4, 4000).unwrap();
    let mut pass = empty_pass();
    let mut allowance = 20_000;
    for _ in 0..4000 {
        pass.enter(&Term::Sort(Universe::Type(0)), &mut allowance)
            .unwrap();
    }
    assert!(pass.certify(&importer, &plan, 4, &mut allowance).unwrap());
}

#[test]
fn a_different_outer_type_cannot_hide_behind_a_long_uniform_suffix() {
    let importer = application(3999);
    let plan = dag::Plan::new(&importer, 4, 4000).unwrap();
    let mut pass = empty_pass();
    let mut allowance = 20_000;
    pass.enter(&Term::Sort(Universe::Type(1)), &mut allowance)
        .unwrap();
    for _ in 1..4000 {
        pass.enter(&Term::Sort(Universe::Type(0)), &mut allowance)
            .unwrap();
    }
    assert!(
        matches!(
            pass.certify(&importer, &plan, 4, &mut allowance),
            Err(Verdict::Reject(_))
        ),
        "the real application requires exact Type0, not Type1"
    );
}

#[test]
fn leaving_a_branch_restores_the_previous_source_typing_environment() {
    let importer = application(0);
    let plan = dag::Plan::new(&importer, 4, 1).unwrap();
    let mut pass = empty_pass();
    pass.enter(&Term::Sort(Universe::Type(0)), &mut 1000)
        .unwrap();
    assert!(pass.certify(&importer, &plan, 4, &mut 1000).unwrap());
    pass.leave();
    pass.enter(&Term::Sort(Universe::Type(1)), &mut 1000)
        .unwrap();
    assert!(matches!(
        pass.certify(&importer, &plan, 4, &mut 1000),
        Err(Verdict::Reject(_))
    ));
    pass.leave();
    pass.enter(&Term::Sort(Universe::Type(0)), &mut 1000)
        .unwrap();
    assert!(pass.certify(&importer, &plan, 4, &mut 1000).unwrap());
}

#[test]
fn an_open_dependent_type_disables_only_its_own_closed_type_branch() {
    let importer = application(0);
    let plan = dag::Plan::new(&importer, 4, 1).unwrap();
    let mut pass = empty_pass();
    pass.enter(&Term::Var("outer_type".into()), &mut 1000)
        .unwrap();
    assert!(
        !pass.certify(&importer, &plan, 4, &mut 1000).unwrap(),
        "a closed-type certificate cannot assume an open dependent domain"
    );
    pass.leave();
    pass.enter(&Term::Sort(Universe::Type(0)), &mut 1000)
        .unwrap();
    assert!(pass.certify(&importer, &plan, 4, &mut 1000).unwrap());
}

#[test]
fn equal_closed_types_are_not_conversion_evidence_for_captured_values() {
    let mut importer = application(0);
    let ty = Term::Sort(Universe::Type(0));
    importer.ctx.add_declaration("left_value", ty.clone());
    importer.ctx.add_declaration("right_value", ty.clone());
    let left = Term::Global("left_value".into());
    let right = Term::Global("right_value".into());
    let plan = dag::Plan::new(&importer, 4, 1).unwrap();
    for value in [&left, &right] {
        let bindings = vec![("argument".into(), ty.clone(), Some(value.clone()))];
        let mut pass =
            Certificates::new(importer.ctx.clone(), &HashSet::new(), &bindings, &mut 1000).unwrap();
        assert!(pass.certify(&importer, &plan, 4, &mut 1000).unwrap());
    }
    assert!(
        !definitionally_equal(&importer.ctx, &left, &right),
        "typing an argument does not equate distinct values of its type"
    );
}
