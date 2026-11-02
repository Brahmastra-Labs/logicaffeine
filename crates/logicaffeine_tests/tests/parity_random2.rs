//! Stdlib-parity — more deterministic seeded random: `randBits` (getrandbits), `randBytes`
//! (randbytes), `randSample` (sample). Values computed from the rng.lg LCG at seed 42.
//! randBits/randBytes: tw ≡ VM ≡ AOT-Rust ≡ C. randSample builds on shuffleSeq (tw/vm/aot_native).
#![allow(unused_imports, dead_code)]

mod common;
use common::{assert_all_tiers, assert_c_output, assert_compiled_equals_interpreted};

/// k random bits as a non-negative Int. py: random.getrandbits
#[test]
fn parity_rand_bits() {
    let src = "## Main\nShow randBits(42, 8).\n";
    assert_all_tiers(src, "157");
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, "157");
}

/// n pseudo-random bytes. py: random.randbytes
#[test]
fn parity_rand_bytes() {
    let src = "## Main\n\
        Show item 1 of randBytes(42, 3).\n\
        Show item 2 of randBytes(42, 3).\n\
        Show item 3 of randBytes(42, 3).\n";
    assert_all_tiers(src, "157\n115\n79");
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, "157\n115\n79");
}

/// k distinct elements (shuffle prefix). py: random.sample
#[test]
fn parity_rand_sample() {
    // shuffleSeq(42,[1,2,3,4,5]) = [3,1,5,4,2]; first 3 = [3,1,5]
    let src = "## Main\n\
        Show item 1 of randSample(42, [1, 2, 3, 4, 5], 3).\n\
        Show item 2 of randSample(42, [1, 2, 3, 4, 5], 3).\n\
        Show item 3 of randSample(42, [1, 2, 3, 4, 5], 3).\n";
    assert_all_tiers(src, "3\n1\n5");
    assert_compiled_equals_interpreted(src);
}
