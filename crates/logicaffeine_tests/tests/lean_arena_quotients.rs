use logicaffeine_kernel::interface::lean as adapter;
#[test]
fn official_quotient_types_and_computation_rules() {
    for (name, input) in [
        (
            "125_quotMkType",
            include_str!("fixtures/lean_arena/125_quotMkType.ndjson"),
        ),
        (
            "126_quotIndType",
            include_str!("fixtures/lean_arena/126_quotIndType.ndjson"),
        ),
        (
            "127_quotLiftType",
            include_str!("fixtures/lean_arena/127_quotLiftType.ndjson"),
        ),
        (
            "128_quotSoundType",
            include_str!("fixtures/lean_arena/128_quotSoundType.ndjson"),
        ),
        (
            "129_quotLiftReduction",
            include_str!("fixtures/lean_arena/129_quotLiftReduction.ndjson"),
        ),
        (
            "130_quotIndReduction",
            include_str!("fixtures/lean_arena/130_quotIndReduction.ndjson"),
        ),
    ] {
        assert!(
            adapter::check_str(input).is_ok(),
            "{name}: {:?}",
            adapter::check_str(input)
        );
    }
}
#[test]
fn quotient_primitives_reject_forged_types_and_names() {
    let original = include_str!("fixtures/lean_arena/125_quotMkType.ndjson");
    for kind in ["type", "ctor", "lift", "ind"] {
        for field in ["type", "name"] {
            let mut rows: Vec<serde_json::Value> = original
                .lines()
                .map(|s| serde_json::from_str(s).unwrap())
                .collect();
            let row = rows.iter_mut().find(|v| v["quot"]["kind"] == kind).unwrap();
            row["quot"][field] = serde_json::json!(0);
            let input: String = rows.into_iter().map(|v| v.to_string() + "\n").collect();
            assert_eq!(
                adapter::check_str(&input).unwrap_err().code(),
                1,
                "{kind} {field}"
            );
        }
    }
}
#[test]
fn quotient_kinds_cannot_replace_each_other() {
    let original = include_str!("fixtures/lean_arena/125_quotMkType.ndjson");
    let mut rows: Vec<serde_json::Value> = original
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    let row = rows
        .iter_mut()
        .find(|v| v["quot"]["kind"] == "lift")
        .unwrap();
    row["quot"]["kind"] = serde_json::json!("ind");
    let input: String = rows.into_iter().map(|v| v.to_string() + "\n").collect();
    assert_eq!(adapter::check_str(&input).unwrap_err().code(), 1);
}

#[test]
fn ordinary_declarations_cannot_impersonate_quotient_primitives() {
    let original = include_str!("fixtures/lean_arena/125_quotMkType.ndjson");
    let mut rows: Vec<serde_json::Value> = original
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    let row = rows
        .iter_mut()
        .find(|v| v["quot"]["kind"] == "type")
        .unwrap();
    let mut declaration = row["quot"].clone();
    declaration.as_object_mut().unwrap().remove("kind");
    declaration["isUnsafe"] = serde_json::json!(false);
    *row = serde_json::json!({"axiom": declaration});
    let input: String = rows.into_iter().map(|v| v.to_string() + "\n").collect();
    assert_eq!(adapter::check_str(&input).unwrap_err().code(), 1);
}
