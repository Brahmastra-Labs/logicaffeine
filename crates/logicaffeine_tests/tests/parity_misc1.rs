#![allow(unused_imports, dead_code)]
mod common;
use common::assert_all_tiers;
#[test]
fn parity_iter_nth() {
    assert_all_tiers(
        "## Main\nShow nthSeq([10, 20, 30], 2).\nShow nthSeq([10, 20, 30], 9).\n",
        "20\n0",
    );
}
#[test]
fn parity_iter_count() {
    assert_all_tiers("## Main\nShow countSeq([1, 2, 3, 4]).\n", "4");
}
#[test]
fn parity_convert_parse_uint() {
    assert_all_tiers("## Main\nShow parseUint(\"42\").\n", "42");
}
#[test]
fn parity_convert_format_uint() {
    assert_all_tiers("## Main\nShow formatUint(255).\n", "255");
}
