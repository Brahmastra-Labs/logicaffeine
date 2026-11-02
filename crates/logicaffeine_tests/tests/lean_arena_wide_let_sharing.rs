//! Wide checked let values must retain sharing during source zeta lowering.
use logicaffeine_kernel::interface::lean as adapter;
use serde_json::{json, Value};

fn input(rounds: usize, malformed: bool) -> String {
    let mut rows = vec![
        json!({"meta":{"format":{"version":"3.1.0"}}}),
        json!({"in":1,"str":{"pre":0,"str":"choose"}}),
        json!({"in":2,"str":{"pre":0,"str":"wide"}}),
        json!({"il":1,"succ":0}),
    ];
    fn expr(rows: &mut Vec<Value>, next: &mut usize, key: &str, value: Value) -> usize {
        let id = *next;
        *next += 1;
        rows.push(json!({"ie":id,key:value}));
        id
    }
    let mut next = 0;
    let ty = expr(&mut rows, &mut next, "sort", json!(1));
    let leaf = expr(&mut rows, &mut next, "sort", json!(0));
    let b0 = expr(&mut rows, &mut next, "bvar", json!(0));
    let b1 = expr(&mut rows, &mut next, "bvar", json!(1));
    let inner = expr(
        &mut rows,
        &mut next,
        "lam",
        json!({"name":0,"type":ty,"body":b1,"binderInfo":"default"}),
    );
    let body = expr(
        &mut rows,
        &mut next,
        "lam",
        json!({"name":0,"type":ty,"body":inner,"binderInfo":"default"}),
    );
    let inner_ty = expr(
        &mut rows,
        &mut next,
        "forallE",
        json!({"name":0,"type":ty,"body":ty,"binderInfo":"default"}),
    );
    let fun_ty = expr(
        &mut rows,
        &mut next,
        "forallE",
        json!({"name":0,"type":ty,"body":inner_ty,"binderInfo":"default"}),
    );
    rows.push(json!({"def":{"name":1,"all":[1],"type":fun_ty,"value":body,"levelParams":[],"safety":"safe","hints":"abbrev"}}));
    let choose = expr(&mut rows, &mut next, "const", json!({"name":1,"us":[]}));
    let partial = expr(&mut rows, &mut next, "app", json!({"fn":choose,"arg":b0}));
    let value = expr(&mut rows, &mut next, "app", json!({"fn":partial,"arg":b0}));
    let mut body = b0;
    for _ in 0..rounds {
        body = expr(
            &mut rows,
            &mut next,
            "letE",
            json!({"name":0,"type":ty,"value":value,"body":body,"nondep":false}),
        );
    }
    body = expr(
        &mut rows,
        &mut next,
        "letE",
        json!({"name":0,"type":ty,"value":if malformed{ty}else{leaf},"body":body,"nondep":false}),
    );
    rows.push(json!({"def":{"name":2,"all":[2],"type":ty,"value":body,"levelParams":[],"safety":"safe","hints":"abbrev"}}));
    rows.into_iter().map(|r| r.to_string() + "\n").collect()
}
#[test]
fn repeated_closed_let_values_keep_a_compact_checked_representation() {
    let (ctx, report) =
        adapter::import_audited(&input(160, false)).expect("compact source let graph");
    assert!(!report.internal_helpers.is_empty());
    assert!(report.declarations.iter().all(|d| d.axioms.is_empty()));
    let body = ctx.get_definition_body("/s:\"wide\"").unwrap();
    assert!(logicaffeine_kernel::term_store::term_depth(body) < 128);
    assert!(logicaffeine_kernel::defeq_for_test(
        &ctx,
        body,
        &logicaffeine_kernel::Term::Sort(logicaffeine_kernel::Universe::Prop)
    ));
}
#[test]
fn sharing_does_not_hide_a_malformed_closed_let_value() {
    assert!(matches!(
        adapter::check_str(&input(160, true)),
        Err(adapter::Verdict::Reject(_))
    ));
}

#[test]
fn closed_value_helper_reports_the_value_source_not_the_enclosing_let() {
    let source = input(160, false);
    let values = source
        .lines()
        .map(|s| serde_json::from_str::<Value>(s).unwrap())
        .filter_map(|r| r.get("letE").map(|v| v["value"].as_u64().unwrap() as usize))
        .collect::<std::collections::HashSet<_>>();
    let (_, report) = adapter::import_audited(&source).unwrap();
    assert!(!report.internal_helpers.is_empty());
    for helper in report.internal_helpers {
        assert!(
            values.contains(&helper.source_expression),
            "helper {} reports unrelated source {}",
            helper.name,
            helper.source_expression
        );
    }
}
