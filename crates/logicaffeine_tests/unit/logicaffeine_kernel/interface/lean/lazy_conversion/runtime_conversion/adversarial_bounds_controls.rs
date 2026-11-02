//! Producer accounting and raw-record bounds are untrusted at replay/readback.
use super::*;
use serde_json::json;
use std::panic::{catch_unwind, AssertUnwindSafe};

fn fixture() -> Importer<'static> {
    let mut importer = Importer::new(false);
    importer
        .ctx
        .add_declaration("A", Term::Sort(Universe::Type(0)));
    importer.names.insert(1, "A".into());
    importer.declarations.insert("A".into(), 0);
    importer.levels.insert(0, Universe::Type(0));
    importer.level_vars.insert(0, HashSet::new());
    importer
        .exprs
        .insert(0, json!({"ie":0,"const":{"name":1,"us":[]}}));
    importer.exprs.insert(1, json!({"ie":1,"sort":0}));
    importer.exprs.insert(2, json!({"ie":2,"bvar":0}));
    importer.exprs.insert(
        3,
        json!({"ie":3,"lam":{
            "name":0,"type":1,"body":2,"binderInfo":"default"
        }}),
    );
    importer
}

fn rejects_without_panicking(
    search: &Search<'_, '_, '_>,
    root: Goal,
    proof: ProofId,
    budget: &mut usize,
) {
    let outcome = catch_unwind(AssertUnwindSafe(|| search.replay(root, proof, budget)));
    assert!(
        outcome.is_ok(),
        "malformed producer state must return an error, not panic"
    );
    assert!(
        outcome.unwrap().is_err(),
        "malformed producer state must not replay successfully"
    );
}

#[test]
fn native_copy_recomputes_text_payload_after_producer_cost_is_poisoned() {
    let importer = fixture();
    let mut budget = 1_000_000;
    let typed = TypedSource::check(&importer, 0, &HashSet::new(), &mut budget).unwrap();
    let mut search = Search::new(&typed, &mut budget).unwrap();
    let literal = Term::Lit(crate::Literal::Text("x".repeat(1024)));
    let id = search.pool.intern(&literal, &mut budget).unwrap();
    let mut ample = 10_000;
    assert_eq!(search.pool.copy(id, &mut ample).unwrap(), literal);
    assert!(ample < 10_000);

    search.pool.costs.insert(id, 0);
    let mut tiny = 1;
    assert!(
        search.pool.copy(id, &mut tiny).is_err(),
        "a zero producer estimate cannot authorize cloning a 1024-byte payload with allowance1"
    );
    assert!(
        tiny <= 1,
        "independent admission must never refill the allowance"
    );
}

#[test]
fn replay_rejects_an_unused_oversized_raw_call_record() {
    let importer = fixture();
    let mut budget = 1_000_000;
    let typed = TypedSource::check(&importer, 0, &HashSet::new(), &mut budget).unwrap();
    let mut search = Search::new(&typed, &mut budget).unwrap();
    let root = Goal::Eval(search.pool.captured(Syntax::Source(0), 0));
    let proof = search.eval(root, &mut budget).unwrap();
    search.replay(root, proof, &mut budget).unwrap();
    let value = search.proofs[proof].value;
    // Bypass producer call(), just as an untrusted trace can supply raw records.
    // The record is unused: independent admission must still enforce its bound.
    search.calls.push(Call {
        function: value,
        arguments: vec![value; 257],
    });
    rejects_without_panicking(&search, root, proof, &mut budget);
    assert!(
        budget > 100_000,
        "a malformed small trace must be rejected before exhausting work"
    );
}

#[test]
fn replay_returns_an_error_for_foreign_out_of_range_native_handles() {
    for corrupt_proof in [false, true] {
        let importer = fixture();
        let mut budget = 1_000_000;
        let typed = TypedSource::check(&importer, 0, &HashSet::new(), &mut budget).unwrap();
        let mut search = Search::new(&typed, &mut budget).unwrap();
        let root = Goal::Eval(search.pool.captured(Syntax::Source(0), 0));
        let proof = search.eval(root, &mut budget).unwrap();
        search.replay(root, proof, &mut budget).unwrap();
        let mut foreign_store = TermStore::new();
        let mut foreign = foreign_store.mk_global("foreign.initial");
        for index in 0..=search.pool.store.len() {
            foreign = foreign_store.mk_global(&format!("foreign.{index}"));
        }
        assert!(foreign.index() >= search.pool.store.len());
        if corrupt_proof {
            search.proofs[proof].rule = Rule::Atom(foreign);
        } else {
            search.values.push(RuntimeValue::Ground(foreign));
        }
        rejects_without_panicking(&search, root, proof, &mut budget);
        assert!(budget > 100_000);
    }
}

#[test]
fn replay_rejects_a_lambda_value_capturing_its_own_binding_environment() {
    let importer = fixture();
    let mut budget = 1_000_000;
    let typed = TypedSource::check(&importer, 0, &HashSet::new(), &mut budget).unwrap();
    let mut search = Search::new(&typed, &mut budget).unwrap();
    let root = Goal::Eval(search.pool.captured(Syntax::Source(0), 0));
    let proof = search.eval(root, &mut budget).unwrap();
    search.replay(root, proof, &mut budget).unwrap();

    let self_environment = search.envs.raw.len();
    let value = search.values.len();
    search.values.push(RuntimeValue::Lambda(Captured {
        node: Syntax::Source(3),
        env: self_environment,
    }));
    let added = search
        .envs
        .append(
            RawBinding {
                parent: 0,
                value,
                name: None,
            },
            &mut budget,
        )
        .unwrap();
    assert_eq!(added, self_environment);
    // Parent pointers alone are acyclic. The captured value closes the cycle.
    rejects_without_panicking(&search, root, proof, &mut budget);
    assert!(budget > 100_000);
}
