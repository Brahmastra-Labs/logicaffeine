use logicaffeine_kernel::interface::lean as adapter;

#[test]
fn original_magma_deep_n21_accepts() {
    adapter::check_str(include_str!(
        "fixtures/lean_arena/magma-list-deep-n21.ndjson"
    ))
    .expect("the entire original magma-list-deep-n21 input must be checked");
}
#[test]
fn original_magma_deep_n36_accepts() {
    adapter::check_str(include_str!(
        "fixtures/lean_arena/magma-list-deep-n36.ndjson"
    ))
    .expect("the entire original magma-list-deep-n36 input must be checked");
}
