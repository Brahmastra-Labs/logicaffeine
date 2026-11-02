use logicaffeine_kernel::interface::lean as adapter;

#[test]
fn original_magma_pair_n21_accepts() {
    adapter::check_str(include_str!(
        "fixtures/lean_arena/magma-list-pair-n21.ndjson"
    ))
    .expect("the entire original magma-list-pair-n21 input must be checked");
}
