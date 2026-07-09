//! Wave 7 keystones: key-first membership `x in xs` → `xs contains x`
//! (Pythonic), and `x not in xs` → its negation. No new AST node — reuses
//! `Expr::Contains`. Does not collide with the `Repeat for x in xs` binder
//! (that `in` is consumed by the loop header, not by expression parsing).

mod common;
use common::assert_compiled_equals_interpreted_eq;

#[test]
fn in_membership_true() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Let xs be [1, 2, 3].
If 2 in xs:
    Show "yes".
Otherwise:
    Show "no".
"#,
        "yes",
    );
}

#[test]
fn in_membership_false() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Let xs be [1, 2, 3].
If 5 in xs:
    Show "yes".
Otherwise:
    Show "no".
"#,
        "no",
    );
}

#[test]
fn not_in_membership() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Let xs be [1, 2, 3].
If 5 not in xs:
    Show "missing".
Otherwise:
    Show "here".
"#,
        "missing",
    );
}

#[test]
fn in_does_not_break_repeat_binder() {
    // The `in` in the Repeat header must still bind the loop, and the `x in xs`
    // in the body must be membership — both in one program.
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Let xs be [1, 2, 3].
Let mutable hits be 0.
Repeat for x in xs:
    If x in xs:
        hits += 1.
Show hits.
"#,
        "3",
    );
}

#[test]
fn in_on_a_set() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Let s be {10, 20, 30}.
If 20 in s:
    Show "member".
Otherwise:
    Show "nope".
"#,
        "member",
    );
}
