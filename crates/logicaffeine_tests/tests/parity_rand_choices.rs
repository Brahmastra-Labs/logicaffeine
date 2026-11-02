//! Stdlib-parity — `randChoices` (Python random.choices with integer weights): k deterministic
//! weighted draws with replacement, chance proportional to weight. All-integer (cumulative-weight
//! scan + integer modulo — no float), so tw ≡ VM ≡ AOT-Rust ≡ C. Values from the rng.lg LCG.
#![allow(unused_imports, dead_code)]

mod common;
use common::{assert_all_tiers, assert_c_output, assert_compiled_equals_interpreted};

/// k weighted draws with replacement. py: random.choices
#[test]
fn parity_rand_choices() {
    // randChoices(42, [10,20,30], weights [1,1,8], 5) = [20,30,30,10,30] — weight 8 on 30 dominates.
    let src = "## Main\n\
        Show item 1 of randChoices(42, [10, 20, 30], [1, 1, 8], 5).\n\
        Show item 2 of randChoices(42, [10, 20, 30], [1, 1, 8], 5).\n\
        Show item 3 of randChoices(42, [10, 20, 30], [1, 1, 8], 5).\n\
        Show item 4 of randChoices(42, [10, 20, 30], [1, 1, 8], 5).\n\
        Show item 5 of randChoices(42, [10, 20, 30], [1, 1, 8], 5).\n";
    assert_all_tiers(src, "20\n30\n30\n10\n30");
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, "20\n30\n30\n10\n30");
}

/// Single-element population: every draw is that element. py: random.choices
#[test]
fn parity_rand_choices_singleton() {
    let src = "## Main\n\
        Show item 1 of randChoices(99, [5], [3], 3).\n\
        Show item 3 of randChoices(99, [5], [3], 3).\n";
    assert_all_tiers(src, "5\n5");
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, "5\n5");
}
