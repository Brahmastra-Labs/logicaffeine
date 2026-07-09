//! Part I correctness: `and`/`or`/`not` are LOGICAL — truthiness in, Bool
//! out, short-circuit — and empty containers are falsy.
//!
//! The audit rows: `6 and 3` was **2** (a silent bitwise pun), `not 0` was
//! **-1** (bitwise complement), and `If items:` was ALWAYS true because
//! every collection was truthy. The ruling: truthiness → Bool with
//! short-circuit (a logic language's `and` yields a truth value); the
//! symbolic `&`/`|`/`~` are the bitwise spellings. Falsy: `false`, `0`,
//! `0.0`, `nothing`, empty Text/List/Map/Set. Everything else is truthy.

mod common;
use common::assert_compiled_equals_interpreted_eq;

// =====================================================================
// Logical words yield Bool
// =====================================================================

#[test]
fn and_on_ints_is_logical() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Show 6 and 3.
"#,
        "true",
    );
}

#[test]
fn and_with_zero_is_false() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Show 0 and 5.
"#,
        "false",
    );
}

#[test]
fn or_on_ints_is_logical() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Show 0 or 7.
"#,
        "true",
    );
}

#[test]
fn not_is_logical_only() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Show not 0.
"#,
        "true",
    );
}

#[test]
fn not_nonzero_is_false() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Show not 3.
"#,
        "false",
    );
}

// =====================================================================
// Short-circuit: the right side must not evaluate when decided
// =====================================================================

#[test]
fn or_short_circuits_past_a_division_by_zero() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Let safe be true.
Show safe or (1 / 0) equals 1.
"#,
        "true",
    );
}

#[test]
fn and_short_circuits_past_a_division_by_zero() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Let blocked be false.
Show blocked and (1 / 0) equals 1.
"#,
        "false",
    );
}

// =====================================================================
// Empty containers are falsy — `If items:` finally means something
// =====================================================================

#[test]
fn empty_list_is_falsy() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Let xs be [] of Int.
If xs:
    Show "has items".
Otherwise:
    Show "empty".
"#,
        "empty",
    );
}

#[test]
fn nonempty_list_is_truthy() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Let xs be [1, 2].
If xs:
    Show "has items".
Otherwise:
    Show "empty".
"#,
        "has items",
    );
}

#[test]
fn empty_text_is_falsy() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Let s be "".
If s:
    Show "text".
Otherwise:
    Show "blank".
"#,
        "blank",
    );
}

#[test]
fn zero_float_is_falsy() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
If 0.0:
    Show "t".
Otherwise:
    Show "f".
"#,
        "f",
    );
}
