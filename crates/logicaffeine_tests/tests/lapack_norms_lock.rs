//! LOGOS-LAPACK matrix norms — `norm1` (max column sum), `normInf` (max row sum), `normMax`
//! (max absolute entry). The three induced/entrywise norms of `DLANGE` beyond the Frobenius
//! `lange`. All exact, so locked with `assert_all_tiers`.

mod common;
use common::assert_all_tiers;

#[test]
fn norm1_is_max_column_sum() {
    // A=[[1,-2],[3,4]]: column sums {4, 6} → 6.
    assert_all_tiers("## Main\nShow norm1(2, 2, [1.0, -2.0, 3.0, 4.0]).\n", "6");
}

#[test]
fn norm_inf_is_max_row_sum() {
    // A=[[1,-2],[3,4]]: row sums {3, 7} → 7.
    assert_all_tiers("## Main\nShow normInf(2, 2, [1.0, -2.0, 3.0, 4.0]).\n", "7");
}

#[test]
fn norm_max_is_largest_absolute_entry() {
    // A=[[1,-2],[3,4]]: max |entry| = 4.
    assert_all_tiers("## Main\nShow normMax(2, 2, [1.0, -2.0, 3.0, 4.0]).\n", "4");
}
