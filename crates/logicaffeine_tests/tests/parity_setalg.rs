#![allow(unused_imports, dead_code)]
mod common;
use common::assert_all_tiers;
#[test]
fn parity_set_union() {
    assert_all_tiers(
        "## Main\nLet r be seqUnion([1, 2, 3], [3, 4, 5]).\nShow length of r.\nShow item 5 of r.\n",
        "5\n5",
    );
}
#[test]
fn parity_set_intersection() {
    assert_all_tiers("## Main\nLet r be seqIntersection([1, 2, 3, 4], [2, 4, 6]).\nShow length of r.\nShow item 1 of r.\nShow item 2 of r.\n", "2\n2\n4");
}
#[test]
fn parity_set_difference() {
    assert_all_tiers("## Main\nLet r be seqDifference([1, 2, 3, 4], [2, 4]).\nShow length of r.\nShow item 2 of r.\n", "2\n3");
}
#[test]
fn parity_set_symdiff() {
    assert_all_tiers(
        "## Main\nLet r be seqSymmetricDifference([1, 2, 3], [3, 4, 5]).\nShow length of r.\n",
        "4",
    );
}
#[test]
fn parity_set_subset() {
    assert_all_tiers(
        "## Main\nShow isSubset([1, 2], [1, 2, 3]).\nShow isSubset([1, 9], [1, 2, 3]).\n",
        "true\nfalse",
    );
}
