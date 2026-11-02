use logicaffeine_kernel::interface::lean as arena;
use serde_json::{json, Value};
const ORIGINAL: &str = include_str!("fixtures/lean_arena/nested-nonuniform-param.ndjson");
#[test]
fn original_nonuniform_nested_parameter_is_rejected_before_feature_gate() {
    let result = arena::check_str(ORIGINAL);
    assert!(
        matches!(result, Err(arena::Verdict::Reject(_))),
        "{result:?}"
    );
}
#[test]
fn corrected_constructor_against_the_forged_recursors_is_rejected() {
    let mut rows: Vec<Value> = ORIGINAL
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let last = rows.pop().unwrap();
    assert_eq!(last["inductive"]["ctors"][0]["type"], json!(61));
    // Fresh expression nodes alter only this constructor, not shared recursor syntax.
    rows.push(json!({"ie":108,"app":{"fn":55,"arg":4}})); // E w, under its parameter
    rows.push(json!({"ie":109,"app":{"fn":39,"arg":108}})); // L (E w)
    rows.push(json!({"ie":110,"forallE":{"binderInfo":"default","name":22,"type":109,"body":59}}));
    rows.push(json!({"ie":111,"forallE":{"binderInfo":"default","name":20,"type":22,"body":110}}));
    let mut last = last;
    last["inductive"]["ctors"][0]["type"] = json!(111);
    rows.push(last);
    let input = rows
        .iter()
        .map(|r| r.to_string() + "\n")
        .collect::<String>();
    // Uniform value-parameter nesting is now reconstructed exactly (W03), so this corrected
    // constructor is judged against the recursors the forged export still carries, which quantify
    // over `E (W.mk false)`: they are not the uniform family's recursors, and the block is rejected.
    let result = arena::check_str(&input);
    assert!(
        matches!(&result,Err(arena::Verdict::Reject(reason)) if reason.code()==arena::DiagnosticCode::InvalidProof),
        "{result:?}"
    );
}
/// The same shape as Lean actually generates it: the uniform family, exported by Lean 4.14.0 with
/// its own recursors, is accepted as a family with its value parameter and both recursors.
#[test]
fn the_original_uniform_value_parameter_family_is_accepted_exactly() {
    let input = include_str!("fixtures/lean_semantics/LF_W03_UniformValueParam.ndjson");
    let p: Value = serde_json::from_str(include_str!(
        "fixtures/lean_semantics/LF_W03_UniformValueParam.provenance.json"
    ))
    .unwrap();
    assert_eq!(
        p["export_sha256"],
        "dbb56673ae2137e9f566569df5721c90e75e61a5cfc67c7126364f02615768d9"
    );
    assert_eq!(p["toolchain"], "leanprover/lean4:v4.14.0");
    let (ctx, report) = arena::import_audited(input).expect("the uniform original");
    let e = r#"/s:"LF_W03V"/s:"E""#;
    assert!(ctx.is_inductive(e));
    assert_eq!(
        (ctx.inductive_num_params(e), ctx.inductive_num_indices(e)),
        (1, 0)
    );
    for rec in ["rec", "rec_1"] {
        assert!(
            ctx.get_universe_poly(&format!("{e}/s:{rec:?}")).is_some(),
            "{rec}"
        );
    }
    let user = report
        .declarations
        .iter()
        .find(|d| d.name == r#"/s:"LF_W03V"/s:"eFalse""#)
        .unwrap();
    assert!(user.axioms.is_empty(), "Lean: eFalse depends on no axioms");
}
