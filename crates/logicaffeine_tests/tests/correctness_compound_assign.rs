//! Wave 7 keystones: compound assignment `x += e` (and `-= *= /= %=`)
//! desugars to `Set x to x <op> e`. Auto-marks the binding mutable (rides
//! the `=`-mutation infrastructure). No new AST node.

mod common;
use common::assert_compiled_equals_interpreted_eq;

#[test]
fn plus_equals() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Let mutable x be 10.
x += 5.
Show x.
"#,
        "15",
    );
}

#[test]
fn minus_equals() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Let mutable x be 10.
x -= 3.
Show x.
"#,
        "7",
    );
}

#[test]
fn star_equals() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Let mutable x be 4.
x *= 3.
Show x.
"#,
        "12",
    );
}

#[test]
fn slash_equals() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Let mutable x be 20.
x /= 4.
Show x.
"#,
        "5",
    );
}

#[test]
fn percent_equals() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Let mutable x be 17.
x %= 5.
Show x.
"#,
        "2",
    );
}

#[test]
fn plus_equals_in_a_loop_accumulates() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Let mutable total be 0.
Repeat for i from 1 to 5:
    total += i.
Show total.
"#,
        "15",
    );
}

#[test]
fn compound_assign_auto_mutable() {
    // No `mutable` keyword — the compound assign marks it (like `=`).
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Let x be 1.
x += 41.
Show x.
"#,
        "42",
    );
}

// ── Place targets: index and field ─────────────────────────────────────

#[test]
fn plus_equals_on_index_target() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Let mutable xs be [10, 20, 30].
Set item 2 of xs to 20.
xs[2] += 5.
Show item 2 of xs.
"#,
        "25",
    );
}

#[test]
fn plus_equals_on_field_target() {
    assert_compiled_equals_interpreted_eq(
        r#"## A Counter has:
    A count: Int.

## Main
Let mutable c be a new Counter with count 40.
c.count += 2.
Show c.count.
"#,
        "42",
    );
}
