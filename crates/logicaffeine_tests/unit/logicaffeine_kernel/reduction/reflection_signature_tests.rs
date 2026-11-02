#[test]
fn every_reflection_schema_agrees_with_the_prelude() {
    let mut ctx = crate::Context::new();
    crate::prelude::StandardLibrary::register(&mut ctx);
    for name in crate::recheck::primitive_rule_names() {
        assert!(ctx.get_global(name).is_some(), "{name}");
        assert!(super::reflection_declaration_agrees(&ctx, name), "{name}");
    }
}
