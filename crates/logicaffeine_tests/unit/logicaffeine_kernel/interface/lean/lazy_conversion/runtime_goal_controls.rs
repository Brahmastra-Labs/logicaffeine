//! Declared-goal controls for the source/runtime bridge. Successful reduction
//! alone cannot discharge the original source's declaration obligation.
use super::*;
use serde_json::json;
use std::collections::BTreeMap;

fn global(name: &str) -> Term {
    Term::Global(name.into())
}
fn sort(level: u32) -> Term {
    Term::Sort(Universe::Type(level))
}

fn source() -> Importer<'static> {
    let mut importer = Importer::new(false);
    importer.levels.insert(1, Universe::Type(0));
    importer.level_vars.insert(1, HashSet::new());
    for (id, name) in [(1, "A"), (2, "B"), (3, "rootA")] {
        importer.names.insert(id, name.into());
        importer.declarations.insert(name.into(), 0);
    }
    importer.ctx.add_declaration("A", sort(0));
    importer.ctx.add_declaration("B", sort(0));
    importer.ctx.declare("rootA", sort(0), global("A")).unwrap();
    for (id, row) in [
        (0, json!({"ie":0,"sort":1})),
        (1, json!({"ie":1,"const":{"name":1,"us":[]}})),
        (2, json!({"ie":2,"const":{"name":2,"us":[]}})),
        (3, json!({"ie":3,"const":{"name":3,"us":[]}})),
        (4, json!({"ie":4,"bvar":0})),
        (
            5,
            json!({"ie":5,"lam":{
            "name":0,"type":0,"body":4,"binderInfo":"default"}}),
        ),
        (6, json!({"ie":6,"app":{"fn":5,"arg":3}})),
    ] {
        importer.exprs.insert(id, row);
    }
    importer
}

fn check(importer: &Importer, root: usize, declared: &Term, allowance: &mut usize) -> Result<Term> {
    checked_body(importer, root, &HashSet::new(), declared, allowance)
}

fn independently_check_goal(ctx: &Context, body: Term, declared: &Term) {
    let witness = Term::Let {
        name: "checked.goal.value".into(),
        ty: Box::new(declared.clone()),
        value: Box::new(body),
        body: Box::new(Term::Var("checked.goal.value".into())),
    };
    // The annotation is checked by each kernel; no main equality result is
    // passed to the independent checker as a substitute for that obligation.
    assert_eq!(lean_type(ctx, &witness).unwrap(), *declared);
    assert_eq!(crate::recheck(ctx, &witness).unwrap(), *declared);
}

#[derive(Debug, PartialEq)]
struct Published {
    declarations: BTreeMap<String, Term>,
    definitions: BTreeMap<String, (Term, Term)>,
    inductives: BTreeMap<String, Term>,
    pending: Vec<(String, String)>,
    refused: Vec<(String, String)>,
    source_declarations: BTreeMap<String, usize>,
    report_declarations: usize,
    report_helpers: usize,
}
fn published(importer: &Importer) -> Published {
    Published {
        declarations: importer
            .ctx
            .iter_declarations()
            .map(|(name, ty)| (name.into(), ty.clone()))
            .collect(),
        definitions: importer
            .ctx
            .iter_definitions()
            .map(|(name, ty, body)| (name.into(), (ty.clone(), body.clone())))
            .collect(),
        inductives: importer
            .ctx
            .iter_inductives()
            .map(|(name, ty)| (name.into(), ty.clone()))
            .collect(),
        pending: importer
            .ctx
            .pending_obligations()
            .into_iter()
            .map(|(name, dependency)| (name.into(), dependency.into()))
            .collect(),
        refused: importer.ctx.refused_declarations().to_vec(),
        source_declarations: importer
            .declarations
            .iter()
            .map(|(name, arity)| (name.clone(), *arity))
            .collect(),
        report_declarations: importer.report.declarations.len(),
        report_helpers: importer.report.internal_helpers.len(),
    }
}

#[test]
fn exact_declared_goal_accepts_the_root_and_its_source_identity_application() {
    let importer = source();
    for root in [3, 6] {
        let mut allowance = 100_000;
        let body = check(&importer, root, &sort(0), &mut allowance)
            .unwrap_or_else(|error| panic!("root {root}: {error:?}; remaining={allowance}"));
        assert!(definitionally_equal(&importer.ctx, &body, &global("A")));
        independently_check_goal(&importer.ctx, body, &sort(0));
        assert!(allowance > 0 && allowance < 100_000);
    }
}

#[test]
fn a_well_typed_source_cannot_be_returned_at_an_unrelated_declared_type() {
    let importer = source();
    assert!(matches!(
        check(&importer, 6, &global("A"), &mut 100_000),
        Err(Verdict::Reject(_))
    ));
}

#[test]
fn cumulative_universe_acceptance_is_not_exact_declared_goal_conversion() {
    let importer = source();
    // rootA has type Type0. Type1 is a cumulative supertype, but Lean's
    // declaration requirement is exact conversion between the inferred types.
    assert!(matches!(
        check(&importer, 3, &sort(1), &mut 100_000),
        Err(Verdict::Reject(_))
    ));
}

#[test]
fn a_correct_reduced_goal_cannot_hide_an_ill_typed_erased_argument() {
    let mut importer = source();
    importer.exprs.insert(
        7,
        json!({"ie":7,"lam":{
        "name":0,"type":0,"body":1,"binderInfo":"default"}}),
    );
    importer
        .exprs
        .insert(8, json!({"ie":8,"app":{"fn":7,"arg":0}}));
    // (lambda ignored : Type0 => A) Type0 reduces to A : Type0, but the
    // ORIGINAL argument Type0 has type Type1 and cannot be erased before typing.
    assert!(matches!(
        check(&importer, 8, &sort(0), &mut 100_000),
        Err(Verdict::Reject(_))
    ));
}

#[test]
fn an_exhausted_goal_attempt_never_refills_the_callers_allowance() {
    let importer = source();
    for initial in [0, 1] {
        let mut allowance = initial;
        assert!(matches!(
            check(&importer, 6, &sort(0), &mut allowance),
            Err(Verdict::Decline(_))
        ));
        assert!(allowance <= initial);
        let remaining = allowance;
        assert!(matches!(
            check(&importer, 6, &sort(0), &mut allowance),
            Err(Verdict::Decline(_))
        ));
        assert!(allowance <= remaining);
    }
}

#[test]
fn checking_a_body_does_not_publish_source_or_kernel_declarations() {
    let importer = source();
    let before = published(&importer);
    let body = check(&importer, 6, &sort(0), &mut 100_000).unwrap();
    independently_check_goal(&importer.ctx, body, &sort(0));
    assert_eq!(published(&importer), before);
    assert!(matches!(
        check(&importer, 6, &global("B"), &mut 100_000),
        Err(Verdict::Reject(_))
    ));
    assert_eq!(published(&importer), before);
    assert!(matches!(
        check(&importer, 6, &sort(0), &mut 0),
        Err(Verdict::Decline(_))
    ));
    assert_eq!(published(&importer), before);
}
