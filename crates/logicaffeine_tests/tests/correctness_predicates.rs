//! Wave 7 keystones (parser tier): English number predicates desugar to
//! modulo comparisons — `x is even` → `x % 2 == 0`, `x is odd` →
//! `x % 2 == 1`, `x is divisible by n` → `x % n == 0`. Pure parser sugar,
//! no new AST node, all engines free.

mod common;
use common::assert_compiled_equals_interpreted_eq;

#[test]
fn is_even_true() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
If 4 is even:
    Show "yes".
Otherwise:
    Show "no".
"#,
        "yes",
    );
}

#[test]
fn is_even_false() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
If 7 is even:
    Show "yes".
Otherwise:
    Show "no".
"#,
        "no",
    );
}

#[test]
fn is_odd_true() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
If 7 is odd:
    Show "yes".
Otherwise:
    Show "no".
"#,
        "yes",
    );
}

#[test]
fn is_divisible_by() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
If 12 is divisible by 3:
    Show "yes".
Otherwise:
    Show "no".
"#,
        "yes",
    );
}

#[test]
fn is_not_divisible_by() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
If 13 is divisible by 5:
    Show "yes".
Otherwise:
    Show "no".
"#,
        "no",
    );
}

#[test]
fn is_even_on_a_variable() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Let n be 10.
If n is even:
    Show "even".
Otherwise:
    Show "odd".
"#,
        "even",
    );
}

// =====================================================================
// `x is between lo and hi` → `lo <= x and x <= hi` (inclusive)
// =====================================================================

#[test]
fn is_between_inside() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Let x be 5.
If x is between 1 and 10:
    Show "in".
Otherwise:
    Show "out".
"#,
        "in",
    );
}

#[test]
fn is_between_is_inclusive_of_bounds() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
If 10 is between 1 and 10:
    Show "in".
Otherwise:
    Show "out".
"#,
        "in",
    );
}

#[test]
fn is_between_outside() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
If 15 is between 1 and 10:
    Show "in".
Otherwise:
    Show "out".
"#,
        "out",
    );
}
