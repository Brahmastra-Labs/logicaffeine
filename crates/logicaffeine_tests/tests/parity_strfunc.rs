#![allow(unused_imports, dead_code)]
mod common;
use common::assert_all_tiers;
/// Whether any char satisfies a predicate. go: strings.ContainsFunc · bytes.ContainsFunc
#[test]
fn parity_str_contains_func() {
    assert_all_tiers("## Main\nShow containsFuncText((c: Int) -> c is less than 50, \"abc\").\nShow containsFuncText((c: Int) -> c is equal to 98, \"abc\").\n", "false\ntrue");
}
/// 1-based index of the first char satisfying a predicate. go: strings.IndexFunc · bytes.IndexFunc
#[test]
fn parity_str_index_func() {
    assert_all_tiers(
        "## Main\nShow indexFuncText((c: Int) -> c is equal to 99, \"abc\").\n",
        "3",
    );
}
/// Trim leading/trailing chars satisfying a predicate. go: strings.TrimFunc · bytes.TrimFunc
#[test]
fn parity_str_trim_func() {
    assert_all_tiers(
        "## Main\nShow trimFuncText((c: Int) -> c is equal to 120, \"xxhixx\").\n",
        "hi",
    );
}
