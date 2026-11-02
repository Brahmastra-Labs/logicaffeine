//! Stdlib-parity — the deterministic seeded random module (LOGOS's determinism policy: an explicit
//! seed, no hidden RNG state, bit-identical across tiers). Built on the rng.lg LCG. Expected values
//! are computed from the LCG (`h = (seed*2654435761 + 1013904223) % 2147483647`).
#![allow(unused_imports, dead_code)]

mod common;
use common::{assert_all_tiers, assert_c_output, assert_compiled_equals_interpreted};

// ---- rng.lg core (tw/vm/aot_native) ---------------------------------------------------------

/// Seeded integer in an inclusive range. py/go: random.randint / rand.Intn-family
#[test]
fn parity_random_int_range() {
    // randInt(7,1,6): h=267601727, 267601727%6=5 -> 1+5 = 6
    let src = "## Main\nShow randInt(7, 1, 6).\n";
    assert_all_tiers(src, "6");
    assert_compiled_equals_interpreted(src);
}

/// Seeded Fisher-Yates shuffle. py: random.shuffle
#[test]
fn parity_random_shuffle() {
    // shuffleSeq(1,[1,2,3,4,5]) = [1,2,5,4,3]
    let src = "## Main\n\
        Show item 3 of shuffleSeq(1, [1, 2, 3, 4, 5]).\n\
        Show item 5 of shuffleSeq(1, [1, 2, 3, 4, 5]).\n";
    assert_all_tiers(src, "5\n3");
    assert_compiled_equals_interpreted(src);
}

// ---- new seeded functions (tw/vm/aot_native/aot_c), seed 42 -> h=831056541 -------------------

/// Deterministic pseudo-random Real in [0,1). py: random.random
#[test]
fn parity_rand_float() {
    let src = "## Main\nShow floor(randFloat(42) * 1000.0).\n";
    assert_all_tiers(src, "386");
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, "386");
}

/// Deterministic pseudo-random Int in [start, stop). py: random.randrange
#[test]
fn parity_rand_range() {
    let src = "## Main\nShow randRange(42, 0, 100).\n";
    assert_all_tiers(src, "41");
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, "41");
}

/// Deterministic pseudo-random element of a sequence. py: random.choice
#[test]
fn parity_rand_choice() {
    let src = "## Main\nShow randChoice(42, [10, 20, 30, 40, 50]).\n";
    assert_all_tiers(src, "20");
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, "20");
}

/// Deterministic pseudo-random Real in [lo, hi). py: random.uniform
#[test]
fn parity_rand_uniform() {
    let src = "## Main\nShow floor(randUniform(42, 0.0, 10.0) * 100.0).\n";
    assert_all_tiers(src, "386");
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, "386");
}
