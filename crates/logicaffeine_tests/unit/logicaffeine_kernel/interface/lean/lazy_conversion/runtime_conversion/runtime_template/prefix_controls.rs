use super::*;

#[test]
fn static_native_argument_prefixes_are_compiled_once() {
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
    assert!(template.program().calls.iter().all(|call|call.arguments.len()<=2),
        "closed native argument prefixes belong in shared ground slots rather than each dynamic call");
    assert!(budget > 0 && budget < 100_000);
}
