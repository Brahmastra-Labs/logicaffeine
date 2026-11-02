use logicaffeine_kernel::interface::lean as adapter;
use serde_json::{json, Value};
fn input(rows: Vec<Value>) -> String {
    std::iter::once(json!({"meta":{"format":{"version":"3.1.0"}}}))
        .chain(rows)
        .map(|r| r.to_string() + "\n")
        .collect()
}
fn proof_rows() -> Vec<Value> {
    vec![
        json!({"in":1,"str":{"pre":0,"str":"P"}}),
        json!({"in":2,"str":{"pre":0,"str":"assumption"}}),
        json!({"in":3,"str":{"pre":0,"str":"lemma"}}),
        json!({"in":4,"str":{"pre":0,"str":"downstream"}}),
        json!({"ie":0,"sort":0}),
        json!({"axiom":{"name":1,"type":0,"levelParams":[],"isUnsafe":false}}),
        json!({"ie":1,"const":{"name":1,"us":[]}}),
        json!({"axiom":{"name":2,"type":1,"levelParams":[],"isUnsafe":false}}),
        json!({"ie":2,"const":{"name":2,"us":[]}}),
        json!({"thm":{"all":[3],"name":3,"type":1,"value":2,"levelParams":[]}}),
        json!({"ie":3,"const":{"name":3,"us":[]}}),
        json!({"thm":{"all":[4],"name":4,"type":1,"value":3,"levelParams":[]}}),
    ]
}
#[test]
fn independent_declaration_audit_records_types_bodies_and_transitive_axioms() {
    let report = adapter::check_str_with_declaration_audit(&input(proof_rows())).unwrap();
    assert_eq!(report.declarations.len(), 4);
    let downstream = &report.declarations[3];
    assert_eq!(downstream.name, r#"/s:"downstream""#);
    assert_eq!(downstream.kind, "thm");
    assert_eq!(downstream.type_expression, 1);
    assert_eq!(downstream.value_expression, Some(3));
    assert_eq!(downstream.axioms, vec![r#"/s:"P""#, r#"/s:"assumption""#]);
    assert!(!downstream.axioms.iter().any(|n| n.contains("lemma")));
}
#[test]
fn failed_declaration_audit_returns_no_partial_report() {
    let mut rows = proof_rows();
    rows.last_mut().unwrap()["thm"]["value"] = json!(0);
    assert_eq!(
        adapter::check_str_with_declaration_audit(&input(rows))
            .unwrap_err()
            .code(),
        1
    );
}
#[test]
fn fast_arena_check_does_not_claim_independent_declaration_auditing() {
    let report = adapter::check_str_with_report(&input(proof_rows())).unwrap();
    assert!(report.declarations.is_empty());
}
#[test]
fn original_export_has_independent_records_for_all_seven_authored_definitions() {
    let input = include_str!("fixtures/lean_conformance/cases_lean414/T00-001.ndjson");
    let report = adapter::check_str_with_declaration_audit(input).unwrap();
    let authored: Vec<_> = report
        .declarations
        .iter()
        .filter(|d| d.name.starts_with(r#"/s:"T00_001"/"#))
        .collect();
    assert_eq!(authored.len(), 7);
    for d in authored {
        assert_eq!(d.kind, "def");
        assert!(d.value_expression.is_some());
        assert!(d.axioms.is_empty(), "{d:?}");
    }
}
