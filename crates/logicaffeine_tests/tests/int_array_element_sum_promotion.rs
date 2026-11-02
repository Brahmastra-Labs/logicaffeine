//! RED→GREEN: summing 3+ elements of a scalarized `[i64; N]` array must compile and run.
//!
//! A function whose return is a fixed-size buffer lowers to `[i64; N]` (a stack array). `infer_numeric_type`
//! typed a direct `item k of d` as `i64` (Index special-case) but a `(item i + item j)` BINARYOP operand as
//! `unknown` — so the inner add entered the exact-int block and promoted to `logos_add_exact` (→ `LogosInt`)
//! while the enclosing add skipped the block and emitted a raw `i64 +`, yielding `LogosInt + i64` (E0369,
//! `cannot add i64 to LogosInt`). The value is a plain integer sum on every other tier; the AOT-Rust tier
//! must agree, so this drives `assert_compiled_equals_interpreted` + the exact value.

#![allow(unused_imports)]
mod common;
use common::{
    assert_all_tiers, assert_c_output, assert_compiled_equals_interpreted, assert_exact_output,
};

/// The minimal trigger: a straight-line fixed buffer scalarizes to `[i64; 3]`; summing all three
/// elements is a 3-term chain `(item1 + item2) + item3` over array indexes. 1+2+3 = 6.
const STRAIGHT_LINE: &str = "\
## To pack () -> Seq of Int:
    Let mutable out be a new Seq of Int.
    Push 1 to out.
    Push 2 to out.
    Push 3 to out.
    Return out.
## Main
Let d be pack().
Show (item 1 of d) + (item 2 of d) + (item 3 of d).
";

#[test]
fn straight_line_fixed_buffer_sum_tiers() {
    assert_all_tiers(STRAIGHT_LINE, "6");
}

#[test]
fn straight_line_fixed_buffer_sum_aot() {
    assert_exact_output(STRAIGHT_LINE, "6");
    assert_compiled_equals_interpreted(STRAIGHT_LINE);
}

#[test]
fn straight_line_fixed_buffer_sum_c() {
    assert_c_output(STRAIGHT_LINE, "6");
}

/// The loop-built form (the streaming-hash digest shape): `digest4` builds `[i64; 4]` over a fixed
/// 2-slot input, and Main sums all four elements — a 4-term chain over the scalarized array.
const LOOP_BUILT: &str = "\
## To digest4 (a: Int) and (b: Int) -> Seq of Int:
    Let mutable h be a new Seq of Int.
    Push a to h.
    Push b to h.
    Let mutable out be a new Seq of Int.
    Repeat for x in h:
        Push x % 256 to out.
        Push (x / 256) % 256 to out.
    Return out.
## Main
Let d be digest4(513, 1027).
Show (item 1 of d) + (item 2 of d) + (item 3 of d) + (item 4 of d).
";

#[test]
fn loop_built_digest_sum_aot() {
    assert_exact_output(LOOP_BUILT, "10");
    assert_compiled_equals_interpreted(LOOP_BUILT);
}
