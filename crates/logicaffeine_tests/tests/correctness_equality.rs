//! Part I correctness: ONE equality, structural and numeric-coherent, on
//! every engine.
//!
//! The rows under test (LANGUAGE_SMELLS.md Part I):
//!   - `[1,2,3] == [1,2,3]` was FALSE — collections never compared equal
//!     (the tree-walker's `compare.rs` catch-all).
//!   - A struct compared `p == p` FALSE interpreted but TRUE compiled.
//!   - `1 == 1.0` was FALSE while `1 <= 1.0` was TRUE, so `a<=b && b>=a`
//!     no longer implied `a==b`.
//!   - `0.1 + 0.2 == 0.3` was TRUE interpreted (epsilon) and FALSE compiled
//!     (bit) — the ruling: bit-exact IEEE everywhere, and approximate
//!     comparison gets its own explicit spelling (`is approximately`).
//!
//! Also here: set display becomes insertion-ordered on EVERY engine (the
//! interpreter already stores sets as insertion vectors; the compiled side
//! must match), which the `{a, b}` literals from Wave 2 make user-visible.

mod common;
use common::assert_compiled_equals_interpreted_eq;

// =====================================================================
// Structural equality for collections
// =====================================================================

#[test]
fn equal_lists_compare_equal() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Show [1, 2, 3] equals [1, 2, 3].
"#,
        "true",
    );
}

#[test]
fn unequal_lists_compare_unequal() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Show [1, 2] equals [1, 3].
"#,
        "false",
    );
}

#[test]
fn nested_lists_compare_structurally() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Show [[1, 2], [3]] equals [[1, 2], [3]].
"#,
        "true",
    );
}

#[test]
fn maps_compare_by_content_not_insertion_order() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Show {"a": 1, "b": 2} equals {"b": 2, "a": 1}.
"#,
        "true",
    );
}

#[test]
fn sets_compare_by_content() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Show {1, 2, 3} equals {3, 1, 2}.
"#,
        "true",
    );
}

#[test]
fn structs_compare_structurally_on_every_engine() {
    assert_compiled_equals_interpreted_eq(
        r#"## A Point has:
    An x: Int.
    A y: Int.

## Main
Let p be a new Point with x 3 and y 4.
Let q be a new Point with x 3 and y 4.
Show p equals q.
"#,
        "true",
    );
}

// =====================================================================
// Numeric coherence: == agrees with the ordering comparisons
// =====================================================================

#[test]
fn int_float_equality_coerces() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Show 1 equals 1.0.
"#,
        "true",
    );
}

#[test]
fn float_equality_is_bit_exact() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Show 0.1 + 0.2 equals 0.3.
"#,
        "false",
    );
}

#[test]
fn is_approximately_is_the_tolerant_spelling() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Show 0.1 + 0.2 is approximately 0.3.
"#,
        "true",
    );
}

#[test]
fn is_approximately_rejects_a_real_difference() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Show 0.1 is approximately 0.2.
"#,
        "false",
    );
}

// =====================================================================
// Numeric comparison is EXACT — mathematical value, never a lossy cast
// (Python's model). 2^53 + 1 is the first integer f64 cannot represent:
// the float literal rounds to 2^53, and `int as f64` would too — an
// as-f64 comparison calls them equal. Exact comparison does not.
// =====================================================================

#[test]
fn large_int_float_equality_is_exact_not_lossy() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Show 9007199254740993 equals 9007199254740993.0.
"#,
        "false",
    );
}

#[test]
fn representable_int_float_equality_holds() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Show 9007199254740992 equals 9007199254740992.0.
"#,
        "true",
    );
}

#[test]
fn large_int_float_ordering_is_exact() {
    // 2^53 + 1 (exact Int) is strictly greater than the float 2^53 the
    // literal rounds to — a lossy as-f64 ordering would say "equal".
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Show 9007199254740993 is greater than 9007199254740993.0.
"#,
        "true",
    );
}

// =====================================================================
// Set display order: insertion-ordered, identical across engines
// =====================================================================

#[test]
fn set_display_follows_insertion_order_everywhere() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Show {3, 1, 2}.
"#,
        "{3, 1, 2}",
    );
}
