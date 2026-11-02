//! Stdlib-parity north-star corpus - domain: statistics
//!
//! Summary statistics over real-valued sequences (statistics.lg), byte-identical on every tier.
//! Mirrors Python's `statistics` module. Float results compared via `floor(x * 100)`.
#![allow(unused_imports, dead_code)]

mod common;
use common::assert_all_tiers;

/// Arithmetic mean. py: statistics.mean / statistics.fmean
#[test]
fn parity_statistics_mean() {
    assert_all_tiers(
        "## Main\nShow floor(meanReal([1.0, 2.0, 3.0, 4.0]) * 100.0).\n",
        "250",
    );
}

/// Population variance. py: statistics.pvariance
#[test]
fn parity_statistics_pvariance() {
    assert_all_tiers(
        "## Main\nShow floor(pvarianceReal([1.0, 2.0, 3.0, 4.0, 5.0]) * 100.0).\n",
        "200",
    );
}

/// Population standard deviation. py: statistics.pstdev
#[test]
fn parity_statistics_pstdev() {
    assert_all_tiers(
        "## Main\nShow floor(pstdevReal([1.0, 2.0, 3.0, 4.0, 5.0]) * 100.0).\n",
        "141",
    );
}

/// Sample variance (n-1). py: statistics.variance
#[test]
fn parity_statistics_variance() {
    assert_all_tiers(
        "## Main\nShow floor(varianceReal([1.0, 2.0, 3.0, 4.0, 5.0]) * 100.0).\n",
        "250",
    );
}

/// Sample standard deviation. py: statistics.stdev
#[test]
fn parity_statistics_stdev() {
    assert_all_tiers(
        "## Main\nShow floor(stdevReal([1.0, 2.0, 3.0, 4.0, 5.0]) * 100.0).\n",
        "158",
    );
}

/// Harmonic mean. py: statistics.harmonic_mean
#[test]
fn parity_statistics_harmonic_mean() {
    assert_all_tiers(
        "## Main\nShow floor(harmonicMeanReal([1.0, 2.0, 4.0]) * 100.0).\n",
        "171",
    );
}

/// Median (middle value, or mean of two middles). py: statistics.median
#[test]
fn parity_statistics_median() {
    assert_all_tiers(
        "## Main\nShow floor(medianReal([3.0, 1.0, 2.0]) * 100.0).\nShow floor(medianReal([1.0, 2.0, 3.0, 4.0]) * 100.0).\n",
        "200\n250",
    );
}
