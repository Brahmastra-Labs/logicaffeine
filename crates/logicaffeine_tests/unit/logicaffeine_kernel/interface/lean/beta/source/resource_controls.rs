//! Resource regressions through the actual source-certificate prerequisite.
//! No test inserts a fabricated typing certificate or bypasses source checking.
use super::*;
use serde_json::json;

fn assert_declined<T>(result: Result<T>) {
    assert!(
        matches!(result, Err(Verdict::Decline(_))),
        "resource exhaustion must conservatively decline"
    );
}

fn named_type_source(name: &str) -> Importer<'static> {
    let mut importer = Importer::new(false);
    importer
        .ctx
        .add_declaration(name, Term::Sort(Universe::Type(0)));
    assert!(
        importer.ctx.get_global(name).is_some(),
        "fixture declaration was published"
    );
    importer.names.insert(1, name.to_owned());
    importer.declarations.insert(name.to_owned(), 0);
    importer.levels.insert(0, Universe::Type(0));
    importer.level_vars.insert(0, HashSet::new());
    importer.exprs.insert(0, json!({"ie":0,"sort":0}));
    importer
        .exprs
        .insert(1, json!({"ie":1,"const":{"name":1,"us":[]}}));
    importer
}

fn lambda(importer: &mut Importer, id: usize, domain: usize, body: usize) {
    importer.exprs.insert(
        id,
        json!({"ie":id,"lam":{
            "name":0,"type":domain,"body":body,"binderInfo":"default"
        }}),
    );
}

#[test]
fn certified_root_readback_charges_hidden_universe_name_payload() {
    let universe_name = format!("source.resource.{}", "u".repeat(1024));
    let mut importer = Importer::new(false);
    importer
        .levels
        .insert(0, Universe::Var(universe_name.clone()));
    importer
        .level_vars
        .insert(0, HashSet::from([universe_name.clone()]));
    importer.exprs.insert(0, json!({"ie":0,"sort":0}));
    let params = HashSet::from([universe_name.clone()]);
    let mut certification_budget = 100_000;
    let mut certificates =
        SourceCertificates::new(&importer, &params, &mut certification_budget).unwrap();
    assert!(certificates.certify(0, &mut certification_budget).unwrap());

    // The inferred type contains one Term node, but its universe owns the name.
    let mut ample = 100_000;
    let expected = certificates.root_type(0, &mut ample).unwrap();
    assert_eq!(
        expected,
        lean_type(
            &importer.ctx,
            &Term::Sort(Universe::Var(universe_name.clone()))
        )
        .unwrap()
    );
    assert!(
        matches!(&expected, Term::Sort(Universe::Succ(level))
        if matches!(level.as_ref(), Universe::Var(name) if name == &universe_name)),
        "the inferred universe must retain the large parameter name"
    );
    let mut tiny = 8;
    assert_declined(certificates.root_type(0, &mut tiny));
    assert!(
        tiny <= 8,
        "failed admission must not refund or reset the budget"
    );
}

#[test]
fn certified_root_readback_charges_repeated_term_name_payloads() {
    let name = format!("source.resource.{}", "A".repeat(1024));
    let mut importer = named_type_source(&name);
    importer.exprs.insert(2, json!({"ie":2,"bvar":0}));
    lambda(&mut importer, 3, 1, 2);
    let mut budget = 100_000;
    let mut certificates =
        SourceCertificates::new(&importer, &HashSet::new(), &mut budget).unwrap();
    assert!(certificates.certify(3, &mut budget).unwrap());

    let mut ample = 100_000;
    let expected = certificates.root_type(3, &mut ample).unwrap();
    let Term::Pi {
        param_type,
        body_type,
        ..
    } = &expected
    else {
        panic!("checked identity must retain its function type");
    };
    let named = |term: &Term| {
        matches!(term,
        Term::Const { name: actual, levels } if actual == &name && levels.is_empty())
            || matches!(term, Term::Global(actual) if actual == &name)
    };
    assert!(
        named(param_type) && named(body_type),
        "both copied type positions must contain the large published name"
    );
    let mut tiny = 256;
    assert_declined(certificates.root_type(3, &mut tiny));
    assert!(
        tiny <= 256,
        "failed admission must not refund or reset the budget"
    );
}

#[test]
fn synthesized_lambda_types_are_depth_admitted_before_rule_readback() {
    // All domains and leaves are independently valid. Only repeated composition
    // makes the result deep; no deeply nested source domain is decoded wholesale.
    let mut importer = named_type_source("source.resource.A");
    let mut root = 1;
    for id in 2..=TYPE_DEPTH + 2 {
        lambda(&mut importer, id, 0, root);
        root = id;
    }
    let mut budget = 4_000_000;
    let mut certificates =
        SourceCertificates::new(&importer, &HashSet::new(), &mut budget).unwrap();
    assert!(
        certificates.certify(32, &mut budget).unwrap(),
        "the same composition below the depth limit must remain supported"
    );
    let mut readback = 100_000;
    assert!(matches!(
        certificates.root_type(32, &mut readback).unwrap(),
        Term::Pi { .. }
    ));
    assert_declined(certificates.certify(root, &mut budget));
    assert!(budget <= 4_000_000);
}

#[test]
fn ordinary_certified_readback_still_succeeds_and_exhaustion_is_monotone() {
    let importer = named_type_source("source.resource.small");
    let mut budget = 100_000;
    let mut certificates =
        SourceCertificates::new(&importer, &HashSet::new(), &mut budget).unwrap();
    assert!(certificates.certify(1, &mut budget).unwrap());
    let mut ample = 1_024;
    assert_eq!(
        certificates.root_type(1, &mut ample).unwrap(),
        Term::Sort(Universe::Type(0))
    );
    assert!(ample < 1_024);
    let mut empty = 0;
    assert_declined(certificates.root_type(1, &mut empty));
    assert_eq!(empty, 0);
}
