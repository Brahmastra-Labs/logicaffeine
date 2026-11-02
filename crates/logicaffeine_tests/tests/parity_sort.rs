//! Stdlib-parity north-star corpus - domain: sort
//!
//! Ordering predicates over sequences (sort.lg), byte-identical on every tier. `sortSeq`
//! (iter.lg) does the sorting; these answer "is this already ascending?" for Int and Real
//! sequences. Mirrors Go's sort.IntsAreSorted / sort.Float64sAreSorted / sort.SliceIsSorted.
#![allow(unused_imports, dead_code)]

mod common;
use common::assert_all_tiers;

/// Whether an integer sequence is in ascending order. go: sort.IntsAreSorted · sort.SliceIsSorted
#[test]
fn parity_sort_is_sorted_int() {
    assert_all_tiers(
        "## Main\nShow isSortedSeq([1, 2, 2, 3]).\nShow isSortedSeq([1, 3, 2]).\nShow isSortedSeq([5]).\n",
        "true\nfalse\ntrue",
    );
}

/// Whether a real sequence is in ascending order. go: sort.Float64sAreSorted
#[test]
fn parity_sort_is_sorted_real() {
    assert_all_tiers(
        "## Main\nShow isSortedReal([1.0, 2.5, 2.5]).\nShow isSortedReal([1.0, 0.5]).\n",
        "true\nfalse",
    );
}
