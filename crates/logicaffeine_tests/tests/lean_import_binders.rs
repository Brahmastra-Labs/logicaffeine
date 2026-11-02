use logicaffeine_kernel::elaborate::ParamKind;
use logicaffeine_kernel::interface::lean as adapter;
use serde_json::{json, Value};

fn fixture(kinds: &[&str], nested: bool) -> String {
    let mut rows = vec![
        json!({"meta":{"format":{"version":"3.1.0"}}}),
        json!({"in":1,"str":{"pre":0,"str":"binderExample"}}),
        json!({"il":1,"succ":0}),
        json!({"ie":0,"sort":1}),
        json!({"ie":1,"bvar":0}),
    ];
    let (mut ty, mut body) = (0, 1);
    let mut id = 2;
    for kind in kinds.iter().rev() {
        rows.push(json!({"ie":id,"forallE":{"name":0,"type":0,"body":ty,"binderInfo":kind}}));
        ty = id;
        id += 1;
        rows.push(json!({"ie":id,"lam":{"name":0,"type":0,"body":body,"binderInfo":"default"}}));
        body = id;
        id += 1;
    }
    if nested {
        rows.push(json!({"ie":id,"forallE":{"name":0,"type":ty,"body":ty,"binderInfo":"default"}}));
        rows.push(json!({"ie":id+1,"lam":{"name":0,"type":ty,"body":1,"binderInfo":"default"}}));
        ty = id;
        body = id + 1;
        id += 2;
    }
    rows.push(json!({"ie":id,"mdata":{"expr":ty,"data":{"source":"original type annotation"}}}));
    rows.push(json!({"def":{"name":1,"type":id,"value":body,"levelParams":[],"all":[1],"hints":"opaque","safety":"safe"}}));
    rows.into_iter().map(|r| r.to_string() + "\n").collect()
}

#[test]
fn declaration_preserves_all_four_type_binder_kinds_through_metadata() {
    let source = fixture(
        &["implicit", "default", "instImplicit", "strictImplicit"],
        false,
    );
    let (ctx, report) = adapter::import_audited(&source).unwrap();
    assert_eq!(
        ctx.binder_kinds(r#"/s:"binderExample""#),
        Some(
            [
                ParamKind::Implicit,
                ParamKind::Explicit,
                ParamKind::Instance,
                ParamKind::StrictImplicit,
            ]
            .as_slice()
        )
    );
    assert_eq!(report.declarations.len(), 1);
    assert!(report.declarations[0].axioms.is_empty());
}

#[test]
fn nested_argument_telescopes_do_not_become_outer_declaration_parameters() {
    let (ctx, _) =
        adapter::import_audited(&fixture(&["strictImplicit", "implicit"], true)).unwrap();
    assert_eq!(
        ctx.binder_kinds(r#"/s:"binderExample""#),
        Some([ParamKind::Explicit].as_slice())
    );
}

#[test]
fn absent_binder_information_is_not_reported_as_known_explicit() {
    let source = fixture(&["strictImplicit", "default"], false);
    let changed: String = source
        .lines()
        .map(|line| {
            let mut r: Value = serde_json::from_str(line).unwrap();
            if let Some(binder) = r.get_mut("forallE") {
                binder.as_object_mut().unwrap().remove("binderInfo");
            }
            r.to_string() + "\n"
        })
        .collect();
    let (ctx, _) = adapter::import_audited(&changed).unwrap();
    assert_eq!(ctx.binder_kinds(r#"/s:"binderExample""#), None);
}

#[test]
fn unknown_binder_kind_is_an_explicit_unsupported_format() {
    let err = adapter::check_str(&fixture(&["futureBinderKind"], false)).unwrap_err();
    assert_eq!(err.code(), 2);
    assert!(format!("{err:?}").contains("binder"));
}

#[test]
fn argument_domain_binders_are_not_parameters_when_result_is_a_sort() {
    let mut rows: Vec<Value> = fixture(&["strictImplicit", "implicit"], true)
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let declaration = rows.last().unwrap()["def"].clone();
    let type_id = declaration["type"].as_u64().unwrap();
    let outer_type = rows.iter().find(|r| r["ie"] == type_id).unwrap()["mdata"]["expr"].clone();
    for row in &mut rows {
        if row["ie"] == outer_type {
            row["forallE"]["body"] = json!(0);
        }
        if row["ie"] == declaration["value"] {
            row["lam"]["body"] = json!(100000);
        }
    }
    rows.insert(5, json!({"ie":100000,"sort":0}));
    let source: String = rows.into_iter().map(|r| r.to_string() + "\n").collect();
    let (ctx, _) = adapter::import_audited(&source).unwrap();
    assert_eq!(
        ctx.binder_kinds(r#"/s:"binderExample""#),
        Some([ParamKind::Explicit].as_slice())
    );
}
