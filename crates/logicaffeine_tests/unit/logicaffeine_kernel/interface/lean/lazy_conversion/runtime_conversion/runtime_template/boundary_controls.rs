use super::*;

#[test]
fn the_original_checked_wrapper_compiles_with_compact_typeclass_instances() {
    let input =
        include_str!("../../../../../../../tests/fixtures/lean_arena/app-lam-original.ndjson");
    let mut importer = Importer::new(false);
    let mut rows = input.lines().enumerate().skip(1).peekable();
    while let Some((line, text)) = rows.next() {
        let row: Value = serde_json::from_str(text).unwrap();
        if rows.peek().is_none() {
            break;
        }
        importer.record(row, line + 1).unwrap();
    }
    let name = importer.name(&serde_json::json!(107)).unwrap();
    assert_eq!(name, "/s:\"wrap2\"");
    let body = importer.ctx.get_definition_body(&name).unwrap();
    let mut budget = 100_000;
    let template = compile(&importer.ctx, body, &mut budget)
        .unwrap_or_else(|e| panic!("original wrapper template: {e:?}; remaining={budget}"))
        .expect("the original checked higher-order wrapper needs a reusable template");
    let rebuilt = reify(template.program(), &mut budget).unwrap();
    native_conversion::NativeCertificates::new(&importer.ctx, &mut budget)
        .unwrap()
        .conversion(body, &rebuilt, template.actual_type(), &mut budget)
        .unwrap();
    assert_eq!(template.program().parameters.len(), 2);
    assert!(budget > 0 && budget < 100_000);
}

#[test]
fn expanded_universe_occurrences_are_bounded_before_template_readback() {
    let constant = Term::Const {
        name: "level_payload".into(),
        levels: vec![Universe::Type(0); 129],
    };
    let mut code = Program {
        parameters: vec![],
        grounds: vec![constant, Term::Global("combine".into())],
        calls: vec![],
        result: TemplateRef::Ground(0),
    };
    assert!(
        reify(&code, &mut 100_000).is_ok(),
        "one admitted129-level occurrence fits"
    );
    code.calls.push(CallInstruction {
        function: TemplateRef::Ground(1),
        arguments: vec![TemplateRef::Ground(0), TemplateRef::Ground(0)],
    });
    code.result = TemplateRef::Call(0);
    // Five term nodes expand two copies of the129-level payload:258 total.
    assert!(
        matches!(reify(&code, &mut 100_000), Err(Verdict::Decline(_))),
        "a small term DAG must not bypass the aggregate256-level readback bound"
    );
}
