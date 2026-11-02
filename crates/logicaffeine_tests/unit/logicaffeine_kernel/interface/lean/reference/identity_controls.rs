//! New small controls: exact kernel identities do not borrow another declaration's Lean spelling.
use super::*;
use serde_json::json;

fn source(duplicate_zero: bool) -> String {
    let mut rows = vec![
        json!({"meta":{"format":{"version":"3.1.0"},"lean":{"version":"4.14.0"}}}),
        json!({"in":1,"str":{"pre":0,"str":"Nat"}}),
        json!({"in":2,"str":{"pre":1,"str":"zero"}}),
        json!({"in":3,"str":{"pre":1,"str":"succ"}}),
        json!({"in":4,"str":{"pre":0,"str":"Zero"}}),
        json!({"in":5,"str":{"pre":0,"str":"Succ"}}),
        json!({"ie":0,"sort":0}),
    ];
    // This is lookup-only syntax; these dummy headers do not claim Nat kernel admission.
    for name in 1..=5 {
        rows.push(json!({"axiom":{"name":name,"levelParams":[],"type":0,"isUnsafe":false}}));
    }
    if duplicate_zero {
        // Distinct name-table rows can spell the SAME complete hierarchical identity.
        rows.push(json!({"in":6,"str":{"pre":1,"str":"zero"}}));
        rows.push(json!({"axiom":{"name":6,"levelParams":[],"type":0,"isUnsafe":false}}));
    }
    rows.iter().map(|row| row.to_string() + "\n").collect()
}

#[test]
fn exact_primitive_identity_is_distinct_from_a_colliding_lean_spelling() {
    let reference = Reference::read(&source(false)).unwrap();
    let inventory = reference.names().unwrap();
    assert_eq!(inventory.len(), 5);
    for (kernel, lean, raw_other) in [
        ("Zero", "Nat.zero", "/s:\"Zero\""),
        ("Succ", "Nat.succ", "/s:\"Succ\""),
    ] {
        let primitive = reference.declaration_kernel(kernel).unwrap();
        assert_eq!(primitive.name, kernel);
        assert_eq!(primitive.lean_name, lean);
        let unrelated = reference.declaration_kernel(raw_other).unwrap();
        assert_eq!(unrelated.name, raw_other);
        assert_eq!(unrelated.lean_name, kernel);
        assert_eq!(reference.source_lean_name(kernel).unwrap(), raw_other);
        assert_eq!(reference.source_lean_name(lean).unwrap(), kernel);
        assert_eq!(
            reference.source_spelling(kernel).unwrap(),
            (kernel.into(), lean.into())
        );
        assert_eq!(reference.declaration(lean).unwrap().name, kernel);
        assert_eq!(reference.declaration(raw_other).unwrap().name, raw_other);
        // Public mixed-name convenience lookup keeps its previous ambiguity refusal.
        assert_eq!(
            reference.declaration(kernel).unwrap_err().reason().code(),
            DiagnosticCode::MalformedExport
        );
        assert_eq!(
            reference
                .source_root_metrics(kernel, false, 8, 8)
                .unwrap()
                .tree_nodes,
            2
        );
        assert_eq!(
            reference
                .preflight_kernel(&[(kernel.into(), false)], 8, 8)
                .unwrap()
                .tree_nodes,
            2
        );
    }
    assert_eq!(
        reference.declaration_kernel("Nat").unwrap().lean_name,
        "Nat"
    );
}

#[test]
fn lean_spelling_lookup_rejects_distinct_hierarchical_names_with_the_same_dotted_spelling() {
    let mut text = source(false);
    for row in [
        json!({"in":6,"str":{"pre":0,"str":"A"}}),
        json!({"in":7,"str":{"pre":6,"str":"B"}}),
        json!({"in":8,"str":{"pre":0,"str":"A.B"}}),
        json!({"axiom":{"name":7,"levelParams":[],"type":0,"isUnsafe":false}}),
        json!({"axiom":{"name":8,"levelParams":[],"type":0,"isUnsafe":false}}),
    ] {
        text.push_str(&(row.to_string() + "\n"));
    }
    let reference = Reference::read(&text).unwrap();
    assert_eq!(reference.names().unwrap().len(), 7);
    for kernel in ["/s:\"A\"/s:\"B\"", "/s:\"A.B\""] {
        assert_eq!(reference.declaration_kernel(kernel).unwrap().name, kernel);
    }
    assert_eq!(
        reference
            .source_lean_name("A.B")
            .unwrap_err()
            .reason()
            .code(),
        DiagnosticCode::MalformedExport
    );
    assert_eq!(
        reference.declaration("A.B").unwrap_err().reason().code(),
        DiagnosticCode::MalformedExport
    );
}

#[test]
fn genuine_duplicate_kernel_identity_is_rejected_before_inventory_collection() {
    let reference = Reference::read(&source(true)).unwrap();
    for error in [
        reference.names().unwrap_err(),
        reference.declaration_kernel("Zero").unwrap_err(),
        reference.source_spelling("Zero").unwrap_err(),
        reference
            .preflight_kernel(&[("Zero".into(), false)], 8, 8)
            .unwrap_err(),
    ] {
        assert_eq!(error.reason().code(), DiagnosticCode::MalformedExport);
    }
}
