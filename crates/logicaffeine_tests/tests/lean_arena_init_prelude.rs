use logicaffeine_kernel::interface::lean as adapter;

#[test]
fn entire_original_init_prelude_passes_both_checkers() {
    adapter::check_str(include_str!("fixtures/lean_arena/init-prelude.ndjson"))
        .expect("every declaration in the original init-prelude input must check");
}
