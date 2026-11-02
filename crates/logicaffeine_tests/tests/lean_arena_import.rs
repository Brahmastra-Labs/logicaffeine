use logicaffeine_kernel::interface::lean as adapter;
use serde_json::{json, Value};

fn input(mut rows: Vec<Value>) -> String {
    rows.insert(0, json!({"meta":{"format":{"version":"3.1.0"}}}));
    rows.into_iter().map(|r| r.to_string() + "\n").collect()
}
fn basic(value: usize) -> Vec<Value> {
    vec![
        json!({"in":1,"str":{"pre":0,"str":"x"}}),
        json!({"il":1,"succ":0}),
        json!({"ie":0,"sort":1}),
        json!({"ie":1,"sort":0}),
        json!({"def":{"all":[1],"hints":"opaque","levelParams":[],"name":1,"safety":"safe","type":0,"value":value}}),
    ]
}
fn outcome(rows: Vec<Value>) -> i32 {
    adapter::check_str(&input(rows)).map_or_else(|e| e.code(), |_| 0)
}

#[test]
fn checks_a_real_definition() {
    assert_eq!(outcome(basic(1)), 0);
}
#[test]
fn rejects_a_sort_mismatch() {
    assert_eq!(outcome(basic(0)), 1);
}
#[test]
fn duplicate_declarations_are_not_replacements() {
    let mut rows = basic(1);
    rows.push(rows.last().unwrap().clone());
    assert_eq!(outcome(rows), 1);
}
#[test]
fn theorem_must_be_a_proposition() {
    let mut rows = basic(1);
    let d = rows.pop().unwrap()["def"].clone();
    rows.push(json!({"thm":d}));
    assert_eq!(outcome(rows), 1);
}
#[test]
fn rejects_free_bound_variables() {
    let mut rows = basic(1);
    rows.insert(4, json!({"ie":2,"bvar":0}));
    rows.last_mut().unwrap()["def"]["value"] = json!(2);
    assert_eq!(outcome(rows), 1);
}
#[test]
fn unsupported_features_decline_instead_of_rejecting() {
    assert_eq!(
        outcome(vec![json!({"inductive":{"types":[],"ctors":[],"recs":[]}})]),
        2
    );
    let mut rows = basic(1);
    rows.last_mut().unwrap()["def"]["safety"] = json!("unsafe");
    assert_eq!(outcome(rows), 2);
}
#[test]
fn malformed_input_is_not_an_invalid_proof_verdict() {
    assert_eq!(adapter::check_str("{}\n").unwrap_err().code(), 2);
    assert_eq!(adapter::check_str("not json").unwrap_err().code(), 2);
}
#[test]
fn universe_parameters_must_be_bound_and_unique() {
    let mut rows = basic(1);
    rows.insert(1, json!({"in":2,"str":{"pre":0,"str":"u"}}));
    rows.insert(3, json!({"il":2,"param":2}));
    rows[5] = json!({"ie":1,"sort":2});
    assert_eq!(outcome(rows.clone()), 1);
    rows.last_mut().unwrap()["def"]["levelParams"] = json!([2, 2]);
    assert_eq!(outcome(rows), 1);
}
#[test]
fn proof_irrelevant_theorem_is_checked() {
    let rows = vec![
        json!({"in":1,"str":{"pre":0,"str":"P"}}),
        json!({"in":2,"str":{"pre":0,"str":"p"}}),
        json!({"in":3,"str":{"pre":0,"str":"proof"}}),
        json!({"ie":0,"sort":0}),
        json!({"axiom":{"name":1,"type":0,"levelParams":[],"isUnsafe":false}}),
        json!({"ie":1,"const":{"name":1,"us":[]}}),
        json!({"axiom":{"name":2,"type":1,"levelParams":[],"isUnsafe":false}}),
        json!({"ie":2,"const":{"name":2,"us":[]}}),
        json!({"thm":{"all":[3],"name":3,"type":1,"value":2,"levelParams":[]}}),
    ];
    assert_eq!(outcome(rows), 0);
}
#[test]
fn reject_self_reference_even_when_context_supports_forward_definitions() {
    let mut rows = basic(1);
    rows.insert(4, json!({"ie":2,"const":{"name":1,"us":[]}}));
    rows.last_mut().unwrap()["def"]["value"] = json!(2);
    assert_eq!(outcome(rows), 1);
}

#[test]
fn applications_cannot_use_logos_universe_cumulativity() {
    let rows = vec![
        json!({"in":1,"str":{"pre":0,"str":"f"}}),
        json!({"in":2,"str":{"pre":0,"str":"bad"}}),
        json!({"il":1,"succ":0}),
        json!({"il":2,"succ":1}),
        json!({"ie":0,"sort":0}),
        json!({"ie":1,"sort":1}),
        json!({"ie":2,"sort":2}),
        json!({"ie":3,"forallE":{"name":0,"type":2,"body":1,"binderInfo":"default"}}),
        json!({"axiom":{"name":1,"type":3,"levelParams":[],"isUnsafe":false}}),
        json!({"ie":4,"const":{"name":1,"us":[]}}),
        json!({"ie":5,"app":{"fn":4,"arg":0}}),
        json!({"def":{"all":[2],"name":2,"type":1,"value":5,"levelParams":[],"safety":"safe","hints":"opaque"}}),
    ];
    assert_eq!(outcome(rows), 1);
}
#[test]
fn cyclic_expression_graph_is_declined_without_recursing() {
    let rows = vec![json!({"ie":0,"app":{"fn":0,"arg":0}})];
    assert_eq!(outcome(rows), 2);
}
