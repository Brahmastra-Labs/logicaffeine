//! Wave 7 KEYSTONE: the dot. `p.x` is field access (≡ `p's x`), `xs.f(a)`
//! is UFCS method syntax (≡ `f(xs, a)` — the SAME AST). Imperative-scoped;
//! prose periods, `Show x.`, decimals, and digit-glue (`5.sqrt`) are all
//! preserved. Every reasonable spelling parses to an existing node.

mod common;
use common::{assert_compiled_equals_interpreted_eq, assert_same_meaning};

const POINT: &str = "## A Point has:\n    An x: Int.\n    A y: Int.\n\n";

// ── Field access: `p.x` ≡ `p's x` ──────────────────────────────────────

#[test]
fn dot_field_access_reads_like_possessive() {
    assert_same_meaning(
        &format!("{POINT}## Main\nLet p be a new Point.\nShow p's x."),
        &format!("{POINT}## Main\nLet p be a new Point.\nShow p.x."),
    );
}

#[test]
fn dot_field_access_value() {
    assert_compiled_equals_interpreted_eq(
        &format!("{POINT}## Main\nLet p be a new Point with x 7 and y 9.\nShow p.x.\nShow p.y."),
        "7\n9",
    );
}

// ── The critical collision: `Show x.` stays a statement ────────────────

#[test]
fn trailing_period_is_not_a_dot() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Let x be 42.
Show x.
"#,
        "42",
    );
}

#[test]
fn period_before_newline_ends_the_statement() {
    // Two statements, each period-terminated — no dot anywhere.
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Let a be 1.
Let b be 2.
Show a.
Show b.
"#,
        "1\n2",
    );
}

// ── Decimals and digit-glue are preserved ──────────────────────────────

#[test]
fn decimal_literal_still_works() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Let pi be 3.14.
Show pi.
"#,
        "3.14",
    );
}

// ── UFCS: `xs.f(a)` ≡ `f(xs, a)` ───────────────────────────────────────

#[test]
fn ufcs_method_call_is_free_function() {
    assert_same_meaning(
        r#"## To double (n: Int) -> Int:
    Return n * 2.

## Main
Show double(21).
"#,
        r#"## To double (n: Int) -> Int:
    Return n * 2.

## Main
Let n be 21.
Show n.double().
"#,
    );
}

#[test]
fn ufcs_method_with_arg() {
    assert_compiled_equals_interpreted_eq(
        r#"## To add (a: Int) and (b: Int) -> Int:
    Return a + b.

## Main
Let x be 10.
Show x.add(5).
"#,
        "15",
    );
}

// ── Field assignment via the dot ───────────────────────────────────────

#[test]
fn dot_field_assignment() {
    assert_compiled_equals_interpreted_eq(
        &format!("{POINT}## Main\nLet mutable p be a new Point.\nSet p.x to 3.\nShow p.x."),
        "3",
    );
}

// ── Edge cases: mixing, chaining, indexed receivers, trailing period ──

#[test]
fn dot_and_possessive_are_interchangeable_in_one_program() {
    assert_compiled_equals_interpreted_eq(
        &format!(
            "{POINT}## Main\nLet p be a new Point with x 3 and y 4.\nShow p's x.\nShow p.y."
        ),
        "3\n4",
    );
}

#[test]
fn dot_field_then_trailing_period_ends_statement() {
    // `Show p.x.` — the first dot is field access, the last is the period.
    assert_compiled_equals_interpreted_eq(
        &format!("{POINT}## Main\nLet p be a new Point with x 5 and y 6.\nShow p.x."),
        "5",
    );
}

#[test]
fn ufcs_on_indexed_receiver() {
    // `items[1].double()` — the `]`-preceded dot is a method on the element.
    assert_compiled_equals_interpreted_eq(
        r#"## To double (n: Int) -> Int:
    Return n * 2.

## Main
Let xs be [10, 20, 30].
Show xs[2].double().
"#,
        "40",
    );
}

#[test]
fn ufcs_on_parenthesized_receiver() {
    assert_compiled_equals_interpreted_eq(
        r#"## To double (n: Int) -> Int:
    Return n * 2.

## Main
Show (5).double().
"#,
        "10",
    );
}

#[test]
fn chained_ufcs_methods() {
    // `x.double().double()` → double(double(x)).
    assert_compiled_equals_interpreted_eq(
        r#"## To double (n: Int) -> Int:
    Return n * 2.

## Main
Let x be 3.
Show x.double().double().
"#,
        "12",
    );
}
