//! Original-source conversion controls. The typing prerequisite and native
//! conversion witnesses must both succeed before a result is returned.
use super::*;
use serde_json::json;
fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn v(n: &str) -> Term {
    Term::Var(n.into())
}
fn ty() -> Term {
    Term::Sort(Universe::Type(0))
}
fn pi(n: &str, a: Term, b: Term) -> Term {
    Term::Pi {
        param: n.into(),
        param_type: Box::new(a),
        body_type: Box::new(b),
    }
}
fn lam(n: &str, a: Term, b: Term) -> Term {
    Term::Lambda {
        param: n.into(),
        param_type: Box::new(a),
        body: Box::new(b),
    }
}
fn source() -> Importer<'static> {
    let mut i = Importer::new(false);
    i.levels.insert(1, Universe::Type(0));
    i.level_vars.insert(1, HashSet::new());
    for (id, n) in [
        (1, "A"),
        (2, "B"),
        (3, "different_wrapper"),
        (4, "shadow_wrapper"),
    ] {
        i.names.insert(id, n.into());
        i.declarations.insert(n.into(), 0);
    }
    i.ctx.add_declaration("A", ty());
    i.ctx.add_declaration("B", ty());
    i.ctx
        .declare(
            "different_wrapper",
            pi("f", pi("x", ty(), ty()), pi("x", ty(), ty())),
            lam(
                "f",
                pi("x", ty(), ty()),
                lam("x", ty(), Term::App(Box::new(v("f")), Box::new(v("x")))),
            ),
        )
        .unwrap();
    i.ctx
        .declare(
            "shadow_wrapper",
            pi("x", ty(), pi("x", ty(), ty())),
            lam("x", ty(), lam("x", ty(), v("x"))),
        )
        .unwrap();
    for (id, row) in [
        (0, json!({"ie":0,"sort":1})),
        (1, json!({"ie":1,"const":{"name":1,"us":[]}})),
        (2, json!({"ie":2,"const":{"name":2,"us":[]}})),
        (3, json!({"ie":3,"bvar":0})),
        (4, json!({"ie":4,"const":{"name":3,"us":[]}})),
        (5, json!({"ie":5,"const":{"name":4,"us":[]}})),
        (6, json!({"ie":6,"bvar":1})),
    ] {
        i.exprs.insert(id, row);
    }
    i
}
fn sl(i: &mut Importer, id: usize, body: usize) {
    i.exprs.insert(
        id,
        json!({"ie":id,"lam":{"name":0,"type":0,"body":body,"binderInfo":"default"}}),
    );
}
fn sa(i: &mut Importer, id: usize, f: usize, a: usize) {
    i.exprs.insert(id, json!({"ie":id,"app":{"fn":f,"arg":a}}));
}
fn run(i: &Importer, root: usize, budget: &mut usize) -> Result<Term> {
    let typed = TypedSource::check(i, root, &HashSet::new(), budget)?;
    runtime_conversion::normalize(&typed, budget)
}
#[test]
fn renamed_native_wrapper_applies_a_source_lambda() {
    let mut i = source();
    sl(&mut i, 7, 3);
    sa(&mut i, 8, 4, 7);
    sa(&mut i, 9, 8, 2);
    assert_eq!(run(&i, 9, &mut 100_000).unwrap(), g("B"));
}
#[test]
fn native_shadowing_resolves_the_nearest_value() {
    let mut i = source();
    sa(&mut i, 7, 5, 1);
    sa(&mut i, 8, 7, 2);
    assert_eq!(run(&i, 8, &mut 100_000).unwrap(), g("B"));
}
#[test]
fn source_capture_survives_crossing_a_native_lambda() {
    let mut i = source();
    sl(&mut i, 7, 6);
    sa(&mut i, 8, 4, 7);
    sa(&mut i, 9, 8, 2);
    sl(&mut i, 10, 9);
    sa(&mut i, 11, 10, 1);
    assert_eq!(run(&i, 11, &mut 100_000).unwrap(), g("A"));
}
#[test]
fn the_same_source_lambda_distinguishes_equal_typed_captures() {
    let mut i = source();
    sl(&mut i, 7, 6);
    sa(&mut i, 8, 4, 7);
    sa(&mut i, 9, 8, 2);
    sl(&mut i, 10, 9);
    sa(&mut i, 11, 10, 1);
    sa(&mut i, 12, 10, 2);
    assert_eq!(run(&i, 11, &mut 100_000).unwrap(), g("A"));
    assert_eq!(run(&i, 12, &mut 100_000).unwrap(), g("B"));
}
#[test]
fn source_typing_rejects_an_erased_bad_argument_before_conversion() {
    let mut i = source();
    sl(&mut i, 7, 1);
    sa(&mut i, 8, 7, 0);
    assert!(matches!(run(&i, 8, &mut 100_000), Err(Verdict::Reject(_))));
}
#[test]
fn runtime_refuses_when_its_allowance_is_empty() {
    let i = source();
    let typed = TypedSource::check(&i, 1, &HashSet::new(), &mut 100_000).unwrap();
    assert!(runtime_conversion::normalize(&typed, &mut 0).is_err());
}
#[test]
#[ignore = "kernel-pending: the app-lam original publishes only through a certified normal form (scripts/run-kernel-pending-tests.sh)"]
fn original_app_lam_converts_without_expanding_its_captured_source() {
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
        i.record(row, line + 1).unwrap();
    }
    let last = final_row.unwrap();
    assert_eq!(last["def"]["value"], json!(24426));
    let mut budget = 100_000;
    let result = run(&i, 24426, &mut budget)
        .unwrap_or_else(|e| panic!("original runtime conversion: {e:?}; remaining={budget}"));
    assert!(budget > 0 && budget < 100_000);
    assert!(definitionally_equal(&i.ctx, &result, &g("Zero")));
    let inferred = lean_type(&i.ctx, &result).unwrap();
    assert!(definitionally_equal(&i.ctx, &inferred, &g("Nat")));
    crate::recheck(&i.ctx, &result).unwrap();
}
