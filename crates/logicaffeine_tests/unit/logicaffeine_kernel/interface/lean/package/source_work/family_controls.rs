use super::*;
#[test]
fn family_guard_preserves_generated_output_depth_admission() {
    let reference = Reference::read(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../logicaffeine_tests/tests/fixtures/lean_semantics/LF_W03_IndexedProjection.ndjson"
    )))
    .unwrap();
    let context = Context::new();
    let mut work = SourceWork::new(&EnvironmentLimits::default()).unwrap();
    work.budget.max_depth = 17;
    let provider = FamilyProjectionReader {
        context: &context,
        reference: &reference,
        work: &mut work,
    };
    assert_eq!(provider.generated_depth_limit(), Some(17));
}
