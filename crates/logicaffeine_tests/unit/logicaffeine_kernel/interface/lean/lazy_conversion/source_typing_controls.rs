//! Source-typing admission controls. These exercise the real prerequisite;
//! no Boolean typing authority or production acceptance shortcut is installed.
use super::*;
use serde_json::json;

fn source() -> Importer<'static> {
    let mut i = Importer::new(false);
    for (level_id, level) in [(1, 0), (2, 1)] {
        i.levels.insert(level_id, Universe::Type(level));
        i.level_vars.insert(level_id, HashSet::new());
    }
    let ty = Term::Sort(Universe::Type(0));
    for (id, name) in [(1, "A"), (2, "B"), (3, "f")] {
        i.names.insert(id, name.into());
        i.declarations.insert(name.into(), 0);
    }
    i.ctx.add_declaration("A", ty.clone());
    i.ctx.add_declaration("B", ty.clone());
    i.ctx.add_declaration("f", arrow(ty.clone(), ty));
    for (id, row) in [
        (0, json!({"ie":0,"sort":1})),
        (1, json!({"ie":1,"const":{"name":1,"us":[]}})),
        (2, json!({"ie":2,"const":{"name":2,"us":[]}})),
        (3, json!({"ie":3,"bvar":0})),
        (4, json!({"ie":4,"sort":2})),
        (5, json!({"ie":5,"const":{"name":3,"us":[]}})),
    ] {
        i.exprs.insert(id, row);
    }
    i
}
fn arrow(domain: Term, codomain: Term) -> Term {
    Term::Pi {
        param: "_".into(),
        param_type: Box::new(domain),
        body_type: Box::new(codomain),
    }
}
fn lambda(i: &mut Importer, id: usize, ty: usize, body: usize) {
    i.exprs.insert(
        id,
        json!({"ie":id,"lam":{
        "name":0,"type":ty,"body":body,"binderInfo":"default"}}),
    );
}
fn forall(i: &mut Importer, id: usize, ty: usize, body: usize) {
    i.exprs.insert(
        id,
        json!({"ie":id,"forallE":{
        "name":0,"type":ty,"body":body,"binderInfo":"default"}}),
    );
}
fn application(i: &mut Importer, id: usize, function: usize, argument: usize) {
    i.exprs
        .insert(id, json!({"ie":id,"app":{"fn":function,"arg":argument}}));
}
fn identity_application(i: &mut Importer) {
    lambda(i, 6, 0, 3);
    application(i, 7, 6, 1);
}

#[test]
fn original_app_lam_source_typing_fits_the_unchanged_allowance() {
    // Exact upstream source, including every original dependency declaration.
    // Only its final definition is held back so this tests the source-typing
    // prerequisite, not the still-unimplemented full conversion/import path.
    let input = include_str!("../../../../../tests/fixtures/lean_arena/app-lam-original.ndjson");
    let mut i = Importer::new(false);
    let mut rows = input.lines().enumerate().skip(1).peekable();
    let mut final_row = None;
    while let Some((line, text)) = rows.next() {
        let row: Value = serde_json::from_str(text).unwrap();
        if rows.peek().is_none() {
            final_row = Some(row);
            break;
        }
        i.record(row, line + 1)
            .unwrap_or_else(|e| panic!("original prefix line {}: {e:?}", line + 1));
    }
    let last = final_row.expect("original final declaration");
    assert_eq!(last["def"]["name"], json!(109));
    assert_eq!(last["def"]["value"], json!(24426));
    assert_eq!(last["def"]["levelParams"], json!([]));
    let mut allowance = 100_000;
    let _typed = TypedSource::check(&i, 24426, &HashSet::new(), &mut allowance)
        .unwrap_or_else(|e| panic!("original source typing: {e:?}; remaining={allowance}"));
    assert!(
        allowance > 0,
        "source typing must leave work allowance for conversion"
    );
    assert!(
        allowance < 100_000,
        "source admission and typing must be charged"
    );
}

#[test]
fn a_generic_four_thousand_binder_captured_dag_fits_the_same_allowance() {
    let mut i = source();
    let ty = Term::Sort(Universe::Type(0));
    let function = arrow(ty.clone(), ty.clone());
    for (id, name, signature) in [
        (
            4,
            "combine",
            arrow(ty.clone(), arrow(ty.clone(), ty.clone())),
        ),
        (5, "twice", arrow(function.clone(), arrow(function, ty))),
    ] {
        i.names.insert(id, name.into());
        i.declarations.insert(name.into(), 0);
        i.ctx.add_declaration(name, signature);
    }
    i.exprs
        .insert(6, json!({"ie":6,"const":{"name":4,"us":[]}}));
    i.exprs
        .insert(7, json!({"ie":7,"const":{"name":5,"us":[]}}));
    let mut next = 8;
    let mut variables = Vec::new();
    for index in 0..4000 {
        i.exprs.insert(next, json!({"ie":next,"bvar":index}));
        variables.push(next);
        next += 1;
    }
    let mut root = variables[0];
    for variable in variables.into_iter().skip(1) {
        application(&mut i, next, 6, root);
        application(&mut i, next + 1, next, variable);
        root = next + 1;
        next += 2;
    }
    for _ in 0..4000 {
        // Both arguments are the identical source lambda node. All original
        // captured variables remain present; this is not an erased toy body.
        lambda(&mut i, next, 0, root);
        application(&mut i, next + 1, 7, next);
        application(&mut i, next + 2, next + 1, next);
        root = next + 2;
        next += 3;
    }
    let mut allowance = 100_000;
    let _typed = TypedSource::check(&i, root, &HashSet::new(), &mut allowance)
        .unwrap_or_else(|e| panic!("generic captured source: {e:?}; remaining={allowance}"));
    assert!(allowance > 0 && allowance < 100_000);
}

#[test]
fn erased_arguments_are_fully_typed_below_a_source_binder() {
    let mut i = source();
    lambda(&mut i, 6, 0, 1); // ignores an argument of Type 0
    application(&mut i, 7, 6, 0); // Type 0 itself has Type 1, not Type 0
    lambda(&mut i, 8, 0, 7);
    assert!(matches!(
        TypedSource::check(&i, 8, &HashSet::new(), &mut 100_000),
        Err(Verdict::Reject(_))
    ));
}

#[test]
fn shared_application_is_rechecked_under_a_different_binder_type() {
    let mut i = source();
    application(&mut i, 6, 5, 3); // shared f #0; f expects Type 0
    lambda(&mut i, 7, 0, 6); // valid occurrence
    lambda(&mut i, 8, 4, 6); // invalid occurrence, #0 has Type 1
    forall(&mut i, 9, 4, 0); // Type 1 -> Type 0
    lambda(&mut i, 10, 9, 7);
    application(&mut i, 11, 10, 8);
    assert!(matches!(
        TypedSource::check(&i, 11, &HashSet::new(), &mut 100_000),
        Err(Verdict::Reject(_))
    ));
}

#[test]
fn shared_variable_can_legitimately_have_two_different_binder_types() {
    let mut i = source();
    lambda(&mut i, 6, 0, 3);
    lambda(&mut i, 7, 4, 3);
    forall(&mut i, 8, 4, 4);
    lambda(&mut i, 9, 8, 6);
    application(&mut i, 10, 9, 7);
    TypedSource::check(&i, 10, &HashSet::new(), &mut 100_000)
        .unwrap_or_else(|e| panic!("separate well-typed source scopes: {e:?}"));
}

#[test]
fn a_lambda_cycle_cannot_evade_admission_by_growing_its_environment() {
    let mut i = source();
    lambda(&mut i, 6, 0, 6);
    let mut allowance = 1000;
    assert!(TypedSource::check(&i, 6, &HashSet::new(), &mut allowance).is_err());
    assert!(allowance < 1000);
}

#[test]
fn an_application_cycle_is_not_a_source_typing_certificate() {
    let mut i = source();
    application(&mut i, 6, 6, 1);
    let mut allowance = 1000;
    assert!(TypedSource::check(&i, 6, &HashSet::new(), &mut allowance).is_err());
    assert!(allowance < 1000);
}

#[test]
fn a_missing_erased_argument_is_still_an_admission_error() {
    let mut i = source();
    lambda(&mut i, 6, 0, 1);
    application(&mut i, 7, 6, 9999);
    assert!(TypedSource::check(&i, 7, &HashSet::new(), &mut 1000).is_err());
}

#[test]
fn failed_admission_never_refunds_or_returns_a_partial_typing_token() {
    let mut i = source();
    identity_application(&mut i);
    for initial in [0, 1, 2] {
        let mut allowance = initial;
        assert!(matches!(
            TypedSource::check(&i, 7, &HashSet::new(), &mut allowance),
            Err(Verdict::Decline(_))
        ));
        assert!(allowance <= initial);
        let exhausted = allowance;
        assert!(TypedSource::check(&i, 7, &HashSet::new(), &mut allowance).is_err());
        assert!(allowance <= exhausted);
    }
    // A failed attempt cannot leave a cache entry that later bypasses typing.
    application(&mut i, 7, 6, 0);
    assert!(matches!(
        TypedSource::check(&i, 7, &HashSet::new(), &mut 100_000),
        Err(Verdict::Reject(_))
    ));
    application(&mut i, 7, 6, 2);
    let typed = TypedSource::check(&i, 7, &HashSet::new(), &mut 100_000).unwrap();
    let trace = produce(&typed, &mut 1000).unwrap();
    let checked = replay(&typed, &trace, &mut 1000).unwrap();
    assert_eq!(
        checked.readback(&mut 100).unwrap(),
        Term::Global("B".into())
    );
}

#[test]
fn replay_readback_uses_the_current_importers_atom_identity() {
    let mut first = source();
    identity_application(&mut first);
    let typed_first = TypedSource::check(&first, 7, &HashSet::new(), &mut 100_000).unwrap();
    let trace = produce(&typed_first, &mut 1000).unwrap();
    let mut second = source();
    identity_application(&mut second);
    second.names.insert(1, "B".into());
    let typed_second = TypedSource::check(&second, 7, &HashSet::new(), &mut 100_000).unwrap();
    // Structurally equivalent traces may replay in another source after its
    // independent typing. An atom ID must resolve in THAT source, never through
    // an old importer cache or a previously materialized A endpoint.
    let checked = replay(&typed_second, &trace, &mut 1000).unwrap();
    assert_eq!(
        checked.readback(&mut 100).unwrap(),
        Term::Global("B".into())
    );
}

#[test]
fn a_second_importer_must_independently_check_its_changed_context() {
    let mut first = source();
    application(&mut first, 6, 5, 1);
    TypedSource::check(&first, 6, &HashSet::new(), &mut 100_000).unwrap();
    let mut second = source();
    application(&mut second, 6, 5, 1);
    // Same source IDs and declaration name, incompatible independently supplied
    // signature. Successful typing in the first importer confers no authority.
    second.ctx.add_declaration(
        "f",
        arrow(Term::Sort(Universe::Type(1)), Term::Sort(Universe::Type(0))),
    );
    assert!(matches!(
        TypedSource::check(&second, 6, &HashSet::new(), &mut 100_000),
        Err(Verdict::Reject(_))
    ));
}

#[test]
fn closed_domain_expanded_size_boundaries_preserve_valid_source_typing() {
    for expanded_size in [255, 256, 257] {
        let mut i = source();
        let mut domain = 0;
        let mut next = 6;
        for _ in 0..7 {
            forall(&mut i, next, domain, domain);
            domain = next;
            next += 1;
        }
        for _ in 255..expanded_size {
            i.exprs
                .insert(next, json!({"ie":next,"mdata":{"data":{},"expr":domain}}));
            domain = next;
            next += 1;
        }
        lambda(&mut i, next, domain, 1);
        // 256/257 is an optimization boundary, not a logical rejection rule.
        // Both independently checked fallback and a bounded graph path are valid.
        TypedSource::check(&i, next, &HashSet::new(), &mut 100_000)
            .unwrap_or_else(|e| panic!("closed domain expanded size {expanded_size}: {e:?}"));
    }
}

#[test]
fn closed_domain_depth_limit_is_conservative_and_does_not_reject_valid_types() {
    for depth in [126, 127, 128] {
        let mut i = source();
        let mut domain = 0;
        let mut next = 6;
        for _ in 1..depth {
            i.exprs
                .insert(next, json!({"ie":next,"mdata":{"data":{},"expr":domain}}));
            domain = next;
            next += 1;
        }
        lambda(&mut i, next, domain, 1);
        let mut allowance = 100_000;
        let outcome = TypedSource::check(&i, next, &HashSet::new(), &mut allowance);
        if depth < 128 {
            outcome.unwrap_or_else(|e| panic!("closed domain depth {depth}: {e:?}"));
        } else {
            // A future bounded implementation may admit this valid source.
            assert!(!matches!(outcome, Err(Verdict::Reject(_))));
        }
        assert!(allowance < 100_000);
    }
}
