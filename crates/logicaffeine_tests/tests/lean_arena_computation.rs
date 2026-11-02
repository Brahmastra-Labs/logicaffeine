use logicaffeine_kernel::interface::lean as adapter;

#[test]
fn checks_the_complete_official_finite_magma_proof() {
    adapter::check_str(include_str!(
        "fixtures/lean_arena/magma-list-pair-n7.ndjson"
    ))
    .unwrap();
}
