#![allow(unused_imports, dead_code)]
mod common;
use common::assert_all_tiers;
#[test]
fn parity_stats_sumprod() {
    assert_all_tiers(
        "## Main\nShow sumProdReal([1.0, 2.0, 3.0], [4.0, 5.0, 6.0]).\n",
        "32",
    );
}
#[test]
fn parity_stats_fsum() {
    assert_all_tiers("## Main\nShow fsumReal([1.5, 2.5, 3.0]).\n", "7");
}
#[test]
fn parity_stats_geomean() {
    assert_all_tiers("## Main\nShow geometricMeanReal([1.0, 2.0, 4.0]).\n", "2");
}
#[test]
fn parity_stats_covariance() {
    assert_all_tiers(
        "## Main\nShow covarianceReal([1.0, 2.0, 3.0], [2.0, 4.0, 6.0]).\n",
        "2",
    );
}
#[test]
fn parity_stats_correlation() {
    assert_all_tiers(
        "## Main\nShow correlationReal([1.0, 2.0, 3.0], [2.0, 4.0, 6.0]).\n",
        "1",
    );
}
/// Ordinary-least-squares slope and intercept. py: statistics.linear_regression
#[test]
fn parity_stats_linear_regression() {
    assert_all_tiers(
        "## Main\nShow linRegSlope([0.0, 1.0, 2.0, 3.0], [1.0, 3.0, 5.0, 7.0]).\nShow linRegIntercept([0.0, 1.0, 2.0, 3.0], [1.0, 3.0, 5.0, 7.0]).\nShow floor(linRegSlope([1.0, 2.0, 3.0, 4.0, 5.0], [2.0, 4.0, 5.0, 4.0, 5.0]) * 10.0 + 0.5).\nShow floor(linRegIntercept([1.0, 2.0, 3.0, 4.0, 5.0], [2.0, 4.0, 5.0, 4.0, 5.0]) * 10.0 + 0.5).\n",
        "2\n1\n6\n22",
    );
}
/// Cut points partitioning data into n equal groups (exclusive method, ×100 to dodge float display). py: statistics.quantiles
#[test]
fn parity_stats_quantiles() {
    assert_all_tiers(
        "## Main\nShow floor(quantile([1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0], 1, 4) * 100.0 + 0.5).\nShow floor(quantile([1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0], 2, 4) * 100.0 + 0.5).\nShow floor(quantile([1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0], 3, 4) * 100.0 + 0.5).\n",
        "275\n550\n825",
    );
}
