//! Check the original ordinary mutual block before rejecting its later invalid projection.
use logicaffeine_kernel::interface::lean as arena;
use serde_json::{json, Value};
const ORIGINAL: &str = include_str!("fixtures/lean_arena/proj-of-imax-prop.ndjson");
fn prefix(change: impl FnOnce(&mut Value)) -> String {
    let mut out = String::new();
    for line in ORIGINAL.lines() {
        let mut row: Value = serde_json::from_str(line).unwrap();
        if row
            .get("inductive")
            .and_then(|d| d["types"].as_array())
            .is_some_and(|ts| ts.len() > 1)
        {
            change(&mut row["inductive"]);
            out.push_str(&row.to_string());
            out.push('\n');
            return out;
        }
        out.push_str(line);
        out.push('\n');
    }
    panic!("original fixture contains the mutual declaration");
}
fn rejects(input: &str) {
    let result = arena::check_str(input);
    assert!(
        matches!(result, Err(arena::Verdict::Reject(_))),
        "{result:?}"
    );
}
#[test]
fn original_valid_mutual_propositions_import_as_one_checked_block() {
    let input = prefix(|_| {});
    let (context, _) = arena::import_audited(&input).unwrap();
    let block = context.mutual_block_of("/s:\"ImaxProp\"").unwrap();
    assert_eq!(
        block,
        &[
            "/s:\"ImaxPropDummy\"".to_string(),
            "/s:\"ImaxProp\"".to_string()
        ]
    );
}
#[test]
fn original_projection_of_semantic_prop_is_rejected_after_mutual_import() {
    rejects(ORIGINAL);
}
#[test]
fn mutual_member_cannot_change_the_common_result_universe() {
    rejects(&prefix(|d| d["types"][1]["type"] = json!(47)));
}
#[test]
fn mutual_member_cannot_omit_a_sibling_from_membership() {
    rejects(&prefix(|d| {
        d["types"][1]["all"].as_array_mut().unwrap().pop();
    }));
}
#[test]
fn mutual_block_requires_every_canonical_recursor() {
    rejects(&prefix(|d| {
        d["recs"].as_array_mut().unwrap().pop();
    }));
}
#[test]
fn mutual_recursor_cannot_forge_k_eligibility() {
    rejects(&prefix(|d| d["recs"][0]["k"] = json!(true)));
}
#[test]
fn mutual_recursor_must_have_every_block_motive() {
    rejects(&prefix(|d| d["recs"][0]["numMotives"] = json!(1)));
}
#[test]
fn mutual_recursor_rule_cannot_name_another_members_constructor() {
    rejects(&prefix(|d| {
        d["recs"][0]["rules"][0]["ctor"] = d["ctors"][0]["name"].clone()
    }));
}

const POLYMORPHIC: &str = include_str!("fixtures/lean_arena/ordinary-mutual-polymorphic.ndjson");

fn polymorphic(change: impl FnOnce(&mut Value)) -> String {
    let mut change = Some(change);
    POLYMORPHIC
        .lines()
        .map(|line| {
            let mut row: Value = serde_json::from_str(line).unwrap();
            if let Some(block) = row.get_mut("inductive") {
                change.take().expect("one independent mutual block")(block);
            }
            row.to_string() + "\n"
        })
        .collect()
}

#[test]
fn original_lean_polymorphic_mutual_recursion_and_definition_are_checked() {
    arena::check_str_with_declaration_audit(POLYMORPHIC).unwrap();
}

#[test]
fn polymorphic_mutual_recursors_accept_export_record_reordering() {
    let input = polymorphic(|d| d["recs"].as_array_mut().unwrap().reverse());
    arena::check_str_with_declaration_audit(&input).unwrap();
}

#[test]
fn polymorphic_mutual_constructors_use_declared_order_not_export_record_order() {
    let input = polymorphic(|d| d["ctors"].as_array_mut().unwrap().reverse());
    arena::check_str_with_declaration_audit(&input).unwrap();
}

#[test]
fn polymorphic_mutual_recursor_cannot_drop_shared_motive_universe() {
    rejects(&polymorphic(|d| {
        d["recs"][0]["levelParams"]
            .as_array_mut()
            .unwrap()
            .remove(0);
    }));
}

#[test]
fn polymorphic_mutual_recursive_rule_cannot_use_leaf_rule() {
    rejects(&polymorphic(|d| {
        d["recs"][0]["rules"][1]["rhs"] = d["recs"][0]["rules"][0]["rhs"].clone();
    }));
}

#[test]
fn polymorphic_mutual_member_cannot_drop_shared_type_parameter() {
    rejects(&polymorphic(|d| d["types"][1]["numParams"] = json!(0)));
}

#[test]
fn polymorphic_mutual_constructor_cannot_drop_shared_type_parameter() {
    rejects(&polymorphic(|d| d["ctors"][3]["numParams"] = json!(0)));
}

#[test]
fn polymorphic_mutual_constructor_order_is_checked_against_header() {
    rejects(&polymorphic(|d| {
        d["types"][0]["ctors"].as_array_mut().unwrap().reverse()
    }));
}
