#![allow(unused_imports, dead_code)]
mod common;
use common::assert_all_tiers;
/// Int -> Real conversion. py: builtins.float
#[test]
fn parity_convert_to_real() {
    assert_all_tiers("## Main\nShow toReal(5) / 2.0.\n", "2.5");
}
/// Floating-point remainder. go: math.Mod · py: math.fmod
#[test]
fn parity_math_fmod() {
    assert_all_tiers(
        "## Main\nShow fmodReal(7.0, 3.0).\nShow fmodReal(8.0, 2.5).\n",
        "1\n0.5",
    );
}
/// Euclidean distance between two points. py: math.dist
#[test]
fn parity_math_dist() {
    assert_all_tiers("## Main\nShow distReal(0.0, 0.0, 3.0, 4.0).\n", "5");
}
/// Mean of an integer sequence (float result). py: statistics.mean
#[test]
fn parity_stats_mean_int() {
    assert_all_tiers("## Main\nShow meanInt([1, 2, 3, 4]).\n", "2.5");
}
