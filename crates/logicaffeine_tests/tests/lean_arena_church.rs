use logicaffeine_kernel::interface::lean as adapter;

#[test]
fn checks_church_numerals_without_materializing_the_deep_normal_form() {
    adapter::check_str(include_str!("fixtures/lean_arena/church-numerals.ndjson")).unwrap();
}
