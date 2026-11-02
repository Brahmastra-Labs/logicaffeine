#![allow(unused_imports, dead_code)]
mod common;
use common::assert_all_tiers;
/// Running (cumulative) sum. py: itertools.accumulate · rust: Iterator.scan
#[test]
fn parity_iter_scan() {
    assert_all_tiers(
        "## Main\nLet r be scanSeq([1, 2, 3, 4]).\nShow item 2 of r.\nShow item 4 of r.\n",
        "3\n10",
    );
}
/// First index where the sequence stops being < x (partition point). rust: slice.partition_point
#[test]
fn parity_iter_partition_point() {
    assert_all_tiers("## Main\nShow partitionPoint([1, 2, 3, 5, 8], 4).\n", "4");
}
