use logicaffeine_kernel::interface::lean as adapter;
use serde_json::{json, Value};

const EMPTY: &str = include_str!("fixtures/lean_arena/036_empty.ndjson");
const BOOL: &str = include_str!("fixtures/lean_arena/037_boolType.ndjson");
const AND: &str = include_str!("fixtures/lean_arena/039_andType.ndjson");
const NAT: &str = include_str!("fixtures/lean_arena/044_natDef.ndjson");
const EMPTY_REC: &str = include_str!("fixtures/lean_arena/062_emptyRec.ndjson");
const BOOL_REC: &str = include_str!("fixtures/lean_arena/063_boolRec.ndjson");
const BOGUS_REC: &str = include_str!("fixtures/lean_arena/073_BogusRecursor.ndjson");
fn changed(source: &str, change: impl FnOnce(&mut Value)) -> String {
    let mut rows: Vec<Value> = source
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    let block = rows
        .iter_mut()
        .find_map(|r| r.get_mut("inductive"))
        .unwrap();
    change(block);
    rows.iter().map(|r| r.to_string() + "\n").collect()
}
fn rejects(s: &str) {
    assert_eq!(adapter::check_str(s).unwrap_err().code(), 1);
}
#[test]
fn imports_empty_and_its_eliminator() {
    for s in [EMPTY, EMPTY_REC] {
        adapter::check_str(s).unwrap();
    }
}
#[test]
fn imports_bool_and_its_eliminator() {
    for s in [BOOL, BOOL_REC] {
        adapter::check_str(s).unwrap();
    }
}
#[test]
fn imports_parameterized_propositions() {
    adapter::check_str(AND).unwrap();
}
#[test]
fn imports_recursive_nat() {
    adapter::check_str(NAT).unwrap();
}
#[test]
fn rejects_a_forged_recursor_type() {
    rejects(&changed(BOOL, |b| b["recs"][0]["type"] = json!(0)));
}
#[test]
fn checks_computation_rules_against_derived_elimination() {
    rejects(&changed(BOOL, |b| {
        let other = b["recs"][0]["rules"][1]["rhs"].clone();
        b["recs"][0]["rules"][0]["rhs"] = other;
    }));
}
#[test]
fn missing_computation_rules_are_not_accepted() {
    rejects(&changed(BOOL, |b| {
        b["recs"][0]["rules"].as_array_mut().unwrap().pop();
    }));
}
#[test]
fn constructor_order_and_field_counts_are_checked() {
    rejects(&changed(BOOL, |b| b["ctors"][1]["cidx"] = json!(0)));
    rejects(&changed(BOOL, |b| b["ctors"][0]["numFields"] = json!(1)));
}
#[test]
fn constructor_names_cannot_alias_a_sibling() {
    rejects(&changed(BOOL, |b| {
        b["ctors"][1]["name"] = b["ctors"][0]["name"].clone()
    }));
}
#[test]
fn a_constructor_cannot_lie_about_its_parameter_count() {
    rejects(&changed(AND, |b| b["ctors"][0]["numParams"] = json!(3)));
}
#[test]
fn recursor_minor_count_is_checked() {
    rejects(&changed(BOOL, |b| b["recs"][0]["numMinors"] = json!(1)));
}
#[test]
fn official_bogus_recursor_is_rejected() {
    rejects(BOGUS_REC);
}

#[test]
fn recursor_names_must_be_the_canonical_generated_names() {
    rejects(include_str!(
        "fixtures/lean_arena/135_misnamed_rec_user.ndjson"
    ));
    rejects(include_str!("fixtures/lean_arena/136_dup_rec_def2.ndjson"));
}
#[test]
fn reducing_a_constructor_index_must_not_strengthen_large_elimination() {
    adapter::check_str(include_str!(
        "fixtures/lean_arena/077_sortElimProp2Rec.ndjson"
    ))
    .unwrap();
}

#[test]
fn recursive_fields_hidden_by_reducible_definitions_0() {
    adapter::check_str(include_str!(
        "fixtures/lean_arena/055_reduceCtorParam.mk.ndjson"
    ))
    .unwrap();
}

#[test]
fn recursive_fields_hidden_by_reducible_definitions_1() {
    adapter::check_str(include_str!(
        "fixtures/lean_arena/118_reduceCtorParamRefl.mk.ndjson"
    ))
    .unwrap();
}

#[test]
fn recursive_fields_hidden_by_reducible_definitions_2() {
    adapter::check_str(include_str!(
        "fixtures/lean_arena/119_reduceCtorParamRefl2.mk.ndjson"
    ))
    .unwrap();
}
