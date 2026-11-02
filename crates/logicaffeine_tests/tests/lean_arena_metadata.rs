use logicaffeine_kernel::interface::lean as adapter;
use serde_json::{json, Value};

fn input(rows: Vec<Value>) -> String {
    std::iter::once(json!({"meta":{"format":{"version":"3.1.0"}}}))
        .chain(rows)
        .map(|r| r.to_string() + "\n")
        .collect()
}
fn base() -> Vec<Value> {
    vec![
        json!({"in":1,"str":{"pre":0,"str":"identity"}}),
        json!({"il":1,"succ":0}),
        json!({"ie":0,"sort":0}),
        json!({"ie":1,"sort":1}),
    ]
}
fn definition(value: usize) -> Value {
    json!({"def":{"all":[1],"name":1,"levelParams":[],"type":1,"value":value,"safety":"safe","hints":"opaque"}})
}
#[test]
fn metadata_is_transparent_and_every_key_has_an_exact_loss_record() {
    let mut rows = base();
    rows.push(json!({"ie":2,"mdata":{"expr":0,"data":{"borrowed":"Lean.DataValue.ofBool true","source":"not executable"}}}));
    rows.push(json!({"ie":3,"mdata":{"expr":2,"data":{"borrowed":"Lean.DataValue.ofBool false"}}}));
    rows.push(definition(3));
    let report = adapter::check_str_with_report(&input(rows)).unwrap();
    assert_eq!(report.metadata_losses.len(), 3);
    assert_eq!(
        report.metadata_losses[0],
        adapter::MetadataLoss {
            expression: 2,
            key: "borrowed".into(),
            value: "Lean.DataValue.ofBool true".into()
        }
    );
    assert_eq!(
        report.metadata_losses[1],
        adapter::MetadataLoss {
            expression: 2,
            key: "source".into(),
            value: "not executable".into()
        }
    );
    assert_eq!(
        report.metadata_losses[2],
        adapter::MetadataLoss {
            expression: 3,
            key: "borrowed".into(),
            value: "Lean.DataValue.ofBool false".into()
        }
    );
}
#[test]
fn metadata_cannot_hide_a_type_mismatch() {
    let mut rows = base();
    rows.push(json!({"ie":2,"mdata":{"expr":1,"data":{"trusted":"true"}}}));
    rows.push(definition(2));
    assert_eq!(adapter::check_str(&input(rows)).unwrap_err().code(), 1);
}
#[test]
fn metadata_preserves_binder_scope() {
    let mut rows = base();
    rows.extend([
        json!({"ie":2,"bvar":0}),
        json!({"ie":3,"mdata":{"expr":2,"data":{"source":"local"}}}),
        json!({"ie":4,"lam":{"name":0,"type":1,"body":3,"binderInfo":"default"}}),
        json!({"ie":5,"forallE":{"name":0,"type":1,"body":1,"binderInfo":"default"}}),
    ]);
    let mut d = definition(4);
    d["def"]["type"] = json!(5);
    rows.push(d);
    assert!(adapter::check_str(&input(rows)).is_ok());
}
#[test]
fn metadata_cannot_bind_a_free_variable() {
    let mut rows = base();
    rows.extend([
        json!({"ie":2,"bvar":0}),
        json!({"ie":3,"mdata":{"expr":2,"data":{"binder":"x"}}}),
        definition(3),
    ]);
    assert_eq!(adapter::check_str(&input(rows)).unwrap_err().code(), 1);
}
#[test]
fn metadata_requires_a_backward_reference_and_the_exporter_data_schema() {
    for metadata in [
        json!({"expr":2,"data":{}}),
        json!({"expr":99,"data":{}}),
        json!({"expr":0,"data":[]}),
        json!({"expr":0,"data":{"source":1}}),
        json!({"expr":0}),
        json!({"expr":0,"data":{},"unknown":"field"}),
    ] {
        let mut rows = base();
        rows.push(json!({"ie":2,"mdata":metadata}));
        rows.push(definition(2));
        assert_eq!(adapter::check_str(&input(rows)).unwrap_err().code(), 2);
    }
}
#[test]
fn deep_metadata_is_decoded_iteratively() {
    let mut rows = base();
    for id in 2..2002 {
        rows.push(json!({"ie":id,"mdata":{"expr":if id==2 {0} else {id-1},"data":{}}}));
    }
    rows.push(definition(2001));
    assert!(adapter::check_str(&input(rows)).is_ok());
}
#[test]
fn unannotated_input_has_no_metadata_losses() {
    let mut rows = base();
    rows.push(definition(0));
    assert!(adapter::check_str_with_report(&input(rows))
        .unwrap()
        .metadata_losses
        .is_empty());
}

#[test]
fn elaborator_have_markers_require_an_explicit_semantic_path() {
    let mut rows = base();
    rows.push(json!({"ie":2,"mdata":{"expr":0,"data":{"letFun":"Lean.DataValue.ofBool true"}}}));
    rows.push(definition(2));
    let e = adapter::check_str_with_report(&input(rows)).unwrap_err();
    assert_eq!(e.code(), 2);
    assert!(format!("{e:?}").contains("letFun"));
}

#[test]
fn loss_json_names_the_contract_category_and_keeps_exported_values() {
    let mut rows = base();
    rows.push(json!({"ie":2,"mdata":{"expr":0,"data":{"pp.nodot":"Lean.DataValue.ofBool true"}}}));
    rows.push(definition(2));
    let report = adapter::check_str_with_report(&input(rows))
        .unwrap()
        .to_json();
    assert_eq!(report["schema"], "logos-lean-import-report/1");
    assert_eq!(
        report["losses"],
        json!([{
            "category":"SOURCE_METADATA","expression":2,"key":"pp.nodot",
            "exported_value":"Lean.DataValue.ofBool true"
        }])
    );
}
#[test]
fn metadata_in_application_heads_and_binder_domains_is_transparent() {
    let mut rows = base();
    rows.extend([
        json!({"ie":2,"mdata":{"expr":1,"data":{"domain":"annotation"}}}),
        json!({"ie":3,"bvar":0}),
        json!({"ie":4,"lam":{"name":0,"type":2,"body":3,"binderInfo":"default"}}),
        json!({"ie":5,"mdata":{"expr":4,"data":{"head":"annotation"}}}),
        json!({"ie":6,"app":{"fn":5,"arg":0}}),
        definition(6),
    ]);
    assert!(adapter::check_str(&input(rows)).is_ok());
}

#[test]
fn original_lean414_de_bruijn_export_imports_with_its_actual_metadata() {
    let input = include_str!("fixtures/lean_conformance/cases_lean414/T00-001.ndjson");
    let report = adapter::check_str_with_report(input).unwrap();
    assert_eq!(
        report.metadata_losses,
        vec![adapter::MetadataLoss {
            expression: 442,
            key: "borrowed".into(),
            value: "Lean.DataValue.ofBool true".into(),
        }]
    );
}
