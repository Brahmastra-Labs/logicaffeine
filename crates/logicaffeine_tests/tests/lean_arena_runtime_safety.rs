//! Unsafe runtime bodies are isolated from the checked mathematical environment.
use logicaffeine_kernel::interface::lean as arena;
use serde_json::{json, Value};
const UNSAFE: &str = include_str!("fixtures/lean_arena/139_falseFromUnsafe.ndjson");
const PARTIAL: &str = include_str!("fixtures/lean_arena/140_falseFromPartial.ndjson");
fn prefix(input: &str) -> Vec<Value> {
    input
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .take_while(|r: &Value| r.get("thm").is_none())
        .collect()
}
fn encode(rows: Vec<Value>) -> String {
    rows.into_iter()
        .map(|r| r.to_string())
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}
fn unrelated_safe_target(input: &str) -> Vec<Value> {
    let mut rows = prefix(input);
    rows.push(json!({"ie":10,"forallE":{"binderInfo":"default","name":7,"type":1,"body":1}}));
    rows.push(json!({"ie":11,"lam":{"binderInfo":"default","name":7,"type":1,"body":5}}));
    rows.push(json!({"thm":{"all":[7],"levelParams":[],"name":7,"type":10,"value":11}}));
    rows
}
fn rejects(input: &str) {
    let result = arena::check_str(input);
    assert!(
        matches!(result, Err(arena::Verdict::Reject(_))),
        "{result:?}"
    );
}
#[test]
fn safe_theorem_cannot_reference_unsafe_runtime_definition() {
    rejects(UNSAFE);
}
#[test]
fn safe_theorem_cannot_reference_partial_runtime_definition() {
    rejects(PARTIAL);
}
#[test]
fn unsafe_only_export_is_inconclusive() {
    let result = arena::check_str(&encode(prefix(UNSAFE)));
    assert!(
        matches!(result, Err(arena::Verdict::Decline(_))),
        "{result:?}"
    );
}
#[test]
fn unrelated_checked_target_reports_runtime_quarantine_without_trusting_it() {
    let input = encode(unrelated_safe_target(UNSAFE));
    let report = arena::check_str_with_report(&input).unwrap().to_json();
    assert_eq!(report["acceptance_scope"], "safe_kernel_environment");
    let runtime = report["runtime_declarations"].as_array().unwrap();
    assert_eq!(runtime.len(), 1);
    assert_eq!(runtime[0]["status"], "quarantined_unchecked");
    assert_eq!(runtime[0]["safety"], "unsafe");
    let (context, audit) = arena::import_audited(&input).unwrap();
    let name = runtime[0]["name"].as_str().unwrap();
    assert!(
        context.get_global(name).is_none(),
        "runtime headers must not become trusted axioms"
    );
    assert!(
        context.get_definition_body(name).is_none(),
        "runtime bodies must not enter the kernel"
    );
    assert_eq!(audit.declarations.len(), 1);
    assert!(audit.declarations[0].axioms.is_empty());
}
#[test]
fn unsafe_reference_in_discarded_let_value_is_rejected() {
    let mut rows = unrelated_safe_target(UNSAFE);
    rows.pop();
    rows.pop();
    rows.push(json!({"ie":11,"letE":{"name":7,"type":1,"value":9,"body":4,"nonDep":true}}));
    rows.push(json!({"ie":12,"lam":{"binderInfo":"default","name":7,"type":1,"body":11}}));
    rows.push(json!({"thm":{"all":[7],"levelParams":[],"name":7,"type":10,"value":12}}));
    rejects(&encode(rows));
}
#[test]
fn unsafe_reference_in_safe_declaration_type_is_rejected() {
    let mut rows = prefix(UNSAFE);
    rows.push(json!({"ie":10,"lam":{"binderInfo":"default","name":7,"type":1,"body":1}}));
    rows.push(json!({"ie":11,"app":{"fn":10,"arg":9}}));
    rows.push(json!({"axiom":{"name":7,"levelParams":[],"type":11,"isUnsafe":false}}));
    rejects(&encode(rows));
}
#[test]
fn unknown_runtime_safety_is_inconclusive() {
    let mut rows = unrelated_safe_target(UNSAFE);
    for r in &mut rows {
        if let Some(d) = r.get_mut("def") {
            d["safety"] = json!("unknown");
        }
    }
    let result = arena::check_str(&encode(rows));
    assert!(
        matches!(result, Err(arena::Verdict::Decline(_))),
        "{result:?}"
    );
}
