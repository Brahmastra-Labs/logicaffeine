mod common;
use common::assert_all_tiers;
#[test]
fn bignum_computed_all_tiers() {
    // operands fit u64; product 10^20 exceeds i64 (9.2e18) — proves promotion to BigInt.
    assert_all_tiers(
        "## Main\nShow 10000000000 * 10000000000.\n",
        "100000000000000000000",
    );
}
