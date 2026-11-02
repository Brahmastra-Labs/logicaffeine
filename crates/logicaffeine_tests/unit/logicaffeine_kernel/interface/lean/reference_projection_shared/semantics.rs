//! New semantic controls for the test-only independent projection adapter.
//! Original fixtures are unchanged; the nested-let case below is a labeled source mutation.
use super::super::reference::Reference;
use super::super::{self as lean, DiagnosticCode, Verdict};
use crate::{BigInt, Context, Literal, Term, Universe};
use serde_json::{json, Value};

const INDEXED: &str =
    include_str!("../../../../../tests/fixtures/lean_semantics/LF_W03_IndexedProjection.ndjson");
const SOURCE: &str =
    include_str!("../../../../../tests/fixtures/lean_semantics/LF_W03_IndexedProjection.lean");
const PROVENANCE: &str = include_str!(
    "../../../../../tests/fixtures/lean_semantics/LF_W03_IndexedProjection.provenance.json"
);
const PROP_FIRST: &str =
    include_str!("../../../../../tests/fixtures/lean_arena/089_projProp1.ndjson");
const PROP_THIRD: &str =
    include_str!("../../../../../tests/fixtures/lean_arena/091_projProp3.ndjson");
const PROP_DEPENDENT: &str =
    include_str!("../../../../../tests/fixtures/lean_arena/093_projProp5.ndjson");

fn spelling(name: &str) -> String {
    name.split('.').map(|part| format!("/s:{part:?}")).collect()
}
fn p(short: &str) -> String {
    spelling(&format!("LF_W03P.{short}"))
}
fn global(name: &str) -> Term {
    Term::Global(name.into())
}
fn app(a: Term, b: Term) -> Term {
    Term::App(Box::new(a), Box::new(b))
}
fn apps(head: Term, args: Vec<Term>) -> Term {
    args.into_iter().fold(head, app)
}
fn lit(n: u64) -> Term {
    Term::Lit(Literal::Nat(BigInt::from_u64(n)))
}
fn numeral(n: u64) -> Term {
    apps(
        Term::Const {
            name: spelling("OfNat.ofNat"),
            levels: vec![Universe::Prop],
        },
        vec![
            global("Nat"),
            lit(n),
            app(global(&spelling("instOfNatNat")), lit(n)),
        ],
    )
}
fn checked_type(ctx: &Context, term: &Term, expected: &Term) {
    let main = crate::infer_type(ctx, term).expect("main kernel checks independently decoded term");
    let second =
        crate::recheck(ctx, term).expect("independent kernel checks independently decoded term");
    assert!(
        crate::defeq_for_test(ctx, &main, expected),
        "main type: {main} vs {expected}"
    );
    assert!(
        crate::defeq_for_test(ctx, &second, expected),
        "second type: {second} vs {expected}"
    );
}
fn decoded(ctx: &Context, source: &str, name: &str) -> (Term, Term) {
    let source = Reference::read(source).unwrap();
    let mut projection = super::ProjectionReader {
        context: ctx,
        max_depth: 2048,
    };
    let declared = source
        .declaration_with(name, &mut projection)
        .expect("independent declared type");
    let body = source
        .value_with(name, &mut projection)
        .expect("independent projected body");
    let expected = ctx
        .get_global(&declared.name)
        .expect("imported declaration type");
    assert!(
        crate::defeq_for_test(ctx, &declared.ty, expected),
        "{name}: source statement agrees"
    );
    checked_type(ctx, &body, &declared.ty);
    let imported_body = ctx
        .get_definition_body(&declared.name)
        .expect("imported transparent body");
    assert!(
        crate::defeq_for_test(ctx, &body, imported_body),
        "{name}: independent lowering agrees"
    );
    (declared.ty, body)
}
fn original() -> lean::ImportedContext {
    let provenance: Value = serde_json::from_str(PROVENANCE).unwrap();
    assert_eq!(
        provenance["source_sha256"],
        "1614d4a6447d0cf49fd5a805634742e4fd52c3f072651730869369c0a73a9067"
    );
    assert_eq!(
        provenance["export_sha256"],
        "89d2ae1e277336d9000418cfac0de118c0ba98b162d4a9383e3b13edda0d3107"
    );
    assert_eq!(provenance["toolchain"], "leanprover/lean4:v4.14.0");
    assert_eq!(
        (
            provenance["compiler_exit"].as_i64(),
            provenance["exporter_exit"].as_i64()
        ),
        (Some(0), Some(0))
    );
    assert!(SOURCE.contains("addDecl"));
    lean::import_audited(INDEXED)
        .expect("authentic indexed-projection witness imports")
        .0
}
fn fin_two(ctx: &Context) -> Term {
    let Term::App(eq_lhs, _) = ctx.get_global(&p("firstAtZero_mk")).unwrap().clone() else {
        panic!("equality")
    };
    let Term::App(_, lhs) = *eq_lhs else {
        panic!("equality lhs")
    };
    let Term::App(_, boxed) = *lhs else {
        panic!("projected constructor")
    };
    let Term::App(_, value) = *boxed else {
        panic!("constructor second field")
    };
    *value
}

#[test]
fn authentic_indexed_parameterized_and_dependent_projections_check_and_compute() {
    let ctx = original();
    for (family, parameters, indices) in [("IBox", 0, 1), ("DBox", 0, 1), ("PBox", 1, 1)] {
        assert_eq!(ctx.inductive_num_params(&p(family)), parameters);
        assert_eq!(ctx.inductive_num_indices(&p(family)), indices);
        assert_eq!(ctx.get_constructors(&p(family)).len(), 1);
    }
    let (_, fixed) = decoded(&ctx, INDEXED, "LF_W03P.firstAtZero");
    let (_, generic) = decoded(&ctx, INDEXED, "LF_W03P.firstAtAnyIndex");
    let (_, dependent) = decoded(&ctx, INDEXED, "LF_W03P.dependentSecond");
    let (_, parameterized) = decoded(&ctx, INDEXED, "LF_W03P.paramFirst");
    // Inspect the INDEPENDENT body's motive: the family index must remain a bound variable.
    let Term::Lambda { body, .. } = &generic else {
        panic!("index lambda")
    };
    let Term::Lambda { body, .. } = body.as_ref() else {
        panic!("major lambda")
    };
    let Term::Match { motive, cases, .. } = body.as_ref() else {
        panic!("checked match")
    };
    assert_eq!(cases.len(), 1);
    let Term::Lambda {
        param: index,
        param_type,
        body,
    } = motive.as_ref()
    else {
        panic!("indexed motive")
    };
    assert_eq!(param_type.as_ref(), &global("Nat"));
    let Term::Lambda {
        param_type: major_type,
        body: result,
        ..
    } = body.as_ref()
    else {
        panic!("major motive")
    };
    assert_eq!(
        major_type.as_ref(),
        &app(global(&p("IBox")), Term::Var(index.clone()))
    );
    assert_eq!(result.as_ref(), &global("Nat"));

    let value = fin_two(&ctx);
    let boxed = apps(global(&p("IBox.mk")), vec![numeral(7), value.clone()]);
    for call in [
        app(fixed, boxed.clone()),
        apps(generic, vec![numeral(0), boxed]),
    ] {
        checked_type(&ctx, &call, &global("Nat"));
        assert!(crate::defeq_for_test(
            &ctx,
            &crate::normalize_checked(&ctx, &call).unwrap(),
            &lit(7)
        ));
    }
    let boxed = apps(
        global(&p("PBox.mk")),
        vec![
            global(&spelling("Bool")),
            global(&spelling("Bool.false")),
            numeral(3),
        ],
    );
    let call = apps(parameterized, vec![global(&spelling("Bool")), boxed]);
    checked_type(&ctx, &call, &global(&spelling("Bool")));
    assert_eq!(
        crate::normalize_checked(&ctx, &call).unwrap(),
        global(&spelling("Bool.false"))
    );

    let boxed = apps(global(&p("DBox.mk")), vec![numeral(2), value.clone()]);
    let second = app(dependent, boxed);
    checked_type(&ctx, &second, &app(global(&spelling("Fin")), numeral(3)));
    assert!(crate::defeq_for_test(&ctx, &second, &value));
    for wrong in [1, 2] {
        let annotated = Term::Let {
            name: "claim".into(),
            ty: Box::new(app(global(&spelling("Fin")), numeral(wrong))),
            value: Box::new(second.clone()),
            body: Box::new(Term::Var("claim".into())),
        };
        assert!(matches!(
            crate::infer_type(&ctx, &annotated),
            Err(crate::KernelError::TypeMismatch { .. })
        ));
        assert!(matches!(
            crate::recheck(&ctx, &annotated),
            Err(crate::ReCheckError::Ill(_))
        ));
    }
}

fn rows() -> Vec<Value> {
    INDEXED
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}
fn text(rows: &[Value]) -> String {
    rows.iter().map(|row| row.to_string() + "\n").collect()
}
fn name_id(rows: &[Value], name: &str) -> u64 {
    let mut prefix = 0;
    for part in name.split('.') {
        prefix = rows
            .iter()
            .find(|row| row["str"]["pre"] == prefix && row["str"]["str"] == part)
            .unwrap()["in"]
            .as_u64()
            .unwrap();
    }
    prefix
}
fn expression(rows: &[Value], id: u64) -> Value {
    rows.iter().find(|row| row["ie"] == id).unwrap().clone()
}

#[test]
fn nested_source_let_aliases_preserve_outer_index_and_project_the_inner_alias() {
    let mut source = rows();
    let name = name_id(&source, "LF_W03P.firstAtAnyIndex");
    let at = source
        .iter()
        .position(|row| row["def"]["name"] == name)
        .unwrap();
    let outer = expression(&source, source[at]["def"]["value"].as_u64().unwrap());
    let inner = expression(&source, outer["lam"]["body"].as_u64().unwrap());
    let family = expression(&source, inner["lam"]["type"].as_u64().unwrap())["app"]["fn"].clone();
    let original_proj = expression(&source, inner["lam"]["body"].as_u64().unwrap());
    let base = source
        .iter()
        .filter_map(|row| row["ie"].as_u64())
        .max()
        .unwrap()
        + 1;
    // Under λi. λb: alias1's type uses bvar1=i. Under alias1, alias2's type uses bvar2=i.
    // Each let value uses bvar0, first b and then alias1; the final projection uses alias2.
    let bvar0 = original_proj["proj"]["struct"].clone();
    let binder = inner["lam"]["name"].clone();
    let additions = vec![
        json!({"ie":base,"bvar":1}),
        json!({"ie":base+1,"bvar":2}),
        json!({"ie":base+2,"app":{"fn":family,"arg":base}}),
        json!({"ie":base+3,"app":{"fn":family,"arg":base+1}}),
        json!({"ie":base+4,"letE":{"name":binder,"type":base+3,"value":bvar0,"body":original_proj["ie"],"nonDep":false}}),
        json!({"ie":base+5,"letE":{"name":binder,"type":base+2,"value":bvar0,"body":base+4,"nonDep":false}}),
        json!({"ie":base+6,"lam":{"name":binder,"type":inner["lam"]["type"],"body":base+5,"binderInfo":"default"}}),
        json!({"ie":base+7,"lam":{"name":outer["lam"]["name"],"type":outer["lam"]["type"],"body":base+6,"binderInfo":"default"}}),
    ];
    source[at]["def"]["value"] = json!(base + 7);
    source.splice(at..at, additions);
    let mutated = text(&source);
    let (ctx, _) =
        lean::import_audited(&mutated).expect("checked indexed witness with nested aliases");
    let (_, body) = decoded(&ctx, &mutated, "LF_W03P.firstAtAnyIndex");
    let original = original();
    assert!(crate::defeq_for_test(
        &original,
        &body,
        original.get_definition_body(&p("firstAtAnyIndex")).unwrap()
    ));
}

#[test]
fn wrong_parent_and_field_index_are_invalid_for_the_reference_adapter_too() {
    let ctx = original();
    for wrong_parent in [false, true] {
        let mut source = rows();
        let name = name_id(&source, "LF_W03P.firstAtAnyIndex");
        let value = source
            .iter()
            .find(|row| row["def"]["name"] == name)
            .unwrap()["def"]["value"]
            .as_u64()
            .unwrap();
        let outer = expression(&source, value);
        let inner = expression(&source, outer["lam"]["body"].as_u64().unwrap());
        let proj = inner["lam"]["body"].as_u64().unwrap();
        let other = name_id(&source, "LF_W03P.DBox");
        let target = source.iter_mut().find(|row| row["ie"] == proj).unwrap();
        if wrong_parent {
            target["proj"]["typeName"] = json!(other);
        } else {
            target["proj"]["idx"] = json!(2);
        }
        let corrupted = text(&source);
        let importer_error = lean::check_str(&corrupted).unwrap_err();
        assert!(matches!(importer_error, Verdict::Reject(_)));
        assert_eq!(importer_error.reason().code(), DiagnosticCode::InvalidProof);
        let reference = Reference::read(&corrupted).unwrap();
        let error = reference
            .value_with(
                "LF_W03P.firstAtAnyIndex",
                &mut super::ProjectionReader {
                    context: &ctx,
                    max_depth: 2048,
                },
            )
            .unwrap_err();
        assert!(matches!(error, Verdict::Reject(_)));
        assert_eq!(error.reason().code(), DiagnosticCode::InvalidProof);
    }
}

#[test]
fn valid_prop_fields_pass_and_early_data_dependency_is_rejected() {
    for (input, name) in [(PROP_FIRST, "projProp1"), (PROP_THIRD, "projProp3")] {
        let (ctx, _) = lean::import_audited(input).expect("original valid Prop projection");
        decoded(&ctx, input, name);
    }
    let importer_error = lean::check_str(PROP_DEPENDENT).unwrap_err();
    assert!(matches!(importer_error, Verdict::Reject(_)));
    assert_eq!(importer_error.reason().code(), DiagnosticCode::InvalidProof);
    // Import only the authentic prefix: the rejected final definition must not enter context.
    let mut records: Vec<&str> = PROP_DEPENDENT.lines().collect();
    let final_record: Value = serde_json::from_str(records.pop().unwrap()).unwrap();
    assert!(final_record.get("def").is_some());
    let prefix = records.join("\n") + "\n";
    let (ctx, _) = lean::import_audited(&prefix).expect("checked Prop structure prefix");
    let reference = Reference::read(PROP_DEPENDENT).unwrap();
    let error = reference
        .value_with(
            "projProp5",
            &mut super::ProjectionReader {
                context: &ctx,
                max_depth: 2048,
            },
        )
        .unwrap_err();
    assert!(matches!(error, Verdict::Reject(_)));
    assert_eq!(error.reason().code(), DiagnosticCode::InvalidProof);
    assert!(
        format!("{error:?}").contains("dependent projection traverses data in a proof"),
        "early Prop guard: {error:?}"
    );
}
