//! Stdlib-parity north-star corpus - domain: decimal
//! LOGOS has a built-in exact-arithmetic `Decimal`/`Rational` tower + `decimal(x)` builtin — no
//! float drift (`decimal(1)/decimal(3)` is the exact `1/3`, not `0.333…`), covering both
//! Python's `decimal.Decimal` and `fractions.Fraction`.
#![allow(unused_imports, dead_code)]
mod common;
use common::assert_all_tiers;
/// Construct an exact decimal; arithmetic never float-drifts. py: decimal.Decimal
#[test]
fn parity_decimal_make() {
    assert_all_tiers(
        "## Main\nShow decimal(5).\nShow decimal(10) + decimal(2).\n",
        "5\n12",
    );
}
/// Exact rational division renders as a fraction. py: fractions.Fraction
#[test]
fn parity_decimal_fraction() {
    assert_all_tiers("## Main\nShow decimal(1) / decimal(3).\n", "1/3");
}
