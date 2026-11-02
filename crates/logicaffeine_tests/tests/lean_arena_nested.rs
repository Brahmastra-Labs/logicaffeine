use logicaffeine_kernel::interface::lean as adapter;
use serde_json::{json, Value};

const PRELUDE: &str = include_str!("fixtures/lean_arena/init-prelude.ndjson");

// Preserve the pinned official prefix through its first nested declaration,
// including the original Array/List recursor signatures and computation rules.
fn nested_prefix(change: impl FnOnce(&mut Value)) -> String {
    let mut lines = Vec::new();
    for raw in PRELUDE.lines() {
        let mut row: Value = serde_json::from_str(raw).unwrap();
        if let Some(block) = row.get_mut("inductive") {
            if block["types"][0]["numNested"].as_u64().unwrap_or(0) > 0 {
                change(block);
                lines.push(row.to_string());
                return lines.join("\n") + "\n";
            }
        }
        lines.push(raw.to_owned());
    }
    panic!("pinned prelude must contain a nested declaration");
}
fn rejects(change: impl FnOnce(&mut Value)) {
    let error = adapter::check_str(&nested_prefix(change)).unwrap_err();
    assert_eq!(
        error.code(),
        1,
        "invalid nested metadata must be rejected: {error:?}"
    );
}
#[test]
fn accepts_official_nested_syntax_with_original_array_and_list_recursors() {
    adapter::check_str(&nested_prefix(|_| {})).unwrap();
}
#[test]
fn rejects_missing_nested_recursor() {
    rejects(|block| {
        block["recs"].as_array_mut().unwrap().pop();
    });
}
#[test]
fn rejects_incorrect_nested_count() {
    rejects(|block| block["types"][0]["numNested"] = json!(3));
}
#[test]
fn rejects_forged_nested_motive_count() {
    rejects(|block| block["recs"][0]["numMotives"] = json!(4));
}
#[test]
fn rejects_forged_nested_k_flag() {
    rejects(|block| block["recs"][0]["k"] = json!(true));
}
#[test]
fn rejects_forged_nested_computation_rule() {
    rejects(|block| {
        block["recs"][0]["rules"][0]["rhs"] = block["types"][0]["type"].clone();
    });
}
