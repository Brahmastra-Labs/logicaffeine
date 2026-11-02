use logicaffeine_kernel::interface::lean as adapter;

#[test]
fn deep_args_before_unfold() {
    let result = adapter::check_str(include_str!(
        "fixtures/lean_arena/args-before-unfold.ndjson"
    ));
    assert!(result.is_ok(), "{result:?}");
}

#[test]
fn deep_folded_constant_first() {
    let result = adapter::check_str(include_str!(
        "fixtures/lean_arena/folded-constant-first.ndjson"
    ));
    assert!(result.is_ok(), "{result:?}");
}

#[test]
fn deep_folded_constant_last() {
    let result = adapter::check_str(include_str!(
        "fixtures/lean_arena/folded-constant-last.ndjson"
    ));
    assert!(result.is_ok(), "{result:?}");
}

#[test]
fn deep_shared_subterm() {
    let result = adapter::check_str(include_str!("fixtures/lean_arena/shared-subterm.ndjson"));
    assert!(result.is_ok(), "{result:?}");
}

#[test]
fn deep_unroll_versus_evaluate() {
    let result = adapter::check_str(include_str!(
        "fixtures/lean_arena/unroll-versus-evaluate.ndjson"
    ));
    assert!(result.is_ok(), "{result:?}");
}

fn telescope(depth: usize, free: bool) -> String {
    use serde_json::json;
    let mut rows = vec![
        json!({"meta":{"format":{"version":"3.1.0"}}}),
        json!({"in":1,"str":{"pre":0,"str":"deepScope"}}),
        json!({"il":1,"succ":0}),
        json!({"il":2,"succ":1}),
        json!({"ie":0,"sort":1}),
        json!({"ie":1,"sort":2}),
        json!({"ie":2,"bvar":if free {depth} else {depth-1}}),
    ];
    let mut body = 2;
    let mut ty = 0;
    let mut next = 3;
    for i in 0..depth {
        let domain = if i == depth - 1 { 0 } else { 1 };
        rows.push(
            json!({"ie":next,"lam":{"name":0,"type":domain,"body":body,"binderInfo":"default"}}),
        );
        body = next;
        next += 1;
        rows.push(
            json!({"ie":next,"forallE":{"name":0,"type":domain,"body":ty,"binderInfo":"default"}}),
        );
        ty = next;
        next += 1;
    }
    rows.push(json!({"def":{"name":1,"type":ty,"value":body,"levelParams":[],"all":[1],"safety":"safe","hints":"opaque"}}));
    rows.into_iter().map(|r| r.to_string() + "\n").collect()
}

#[test]
fn deep_binder_scope_resolves_outer_variable_at_its_own_type() {
    let result = adapter::check_str(&telescope(512, false));
    assert!(result.is_ok(), "{result:?}");
}

#[test]
fn deep_binder_scope_rejects_an_out_of_scope_variable() {
    assert_eq!(
        adapter::check_str(&telescope(512, true))
            .unwrap_err()
            .code(),
        1
    );
}
