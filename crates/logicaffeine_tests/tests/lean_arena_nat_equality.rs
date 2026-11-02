use logicaffeine_kernel::interface::lean as adapter;
#[test]
fn original_usize_size_cases_are_checked_by_both_kernels() {
    // This original exported proof computes both 2^32 and 2^64 through
    // of_decide_eq_true / Nat.decEq / Nat.beq, including opaque registration.
    let input = include_str!("fixtures/lean_arena/init-prelude.ndjson")
        .lines()
        .take(42210)
        .collect::<Vec<_>>()
        .join("\n");
    adapter::check_str(&input).expect("the original USize.size_eq proof must check");
}
