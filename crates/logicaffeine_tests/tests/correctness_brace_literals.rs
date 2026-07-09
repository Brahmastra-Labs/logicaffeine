//! Part III keystone: `{…}` map and set literals.
//!
//! Today `{`/`}` outside a string are silently dropped by the lexer, so there
//! is no literal syntax for maps or sets — every map is a `new Map` plus one
//! `Set item` statement per entry (the audit's Maps/dicts table, row 1). The
//! spec: `{k: v, …}` (a `:` after the first element) is a map literal,
//! `{a, b, c}` is a set literal, and the empty form requires an element type
//! exactly like the list rule — `{} of Int` (set) / `{} of Text to Int` (map)
//! — threading the intentional empty-literal ambiguity, not flattening it.
//! Both lower to variadic builtins (`mapOf`/`setOf`), so every engine shares
//! one construction path. String interpolation braces are unaffected (they
//! are consumed inside the string-literal lexer path).

mod common;
use common::{assert_compiled_equals_interpreted_eq, assert_same_meaning};

// =====================================================================
// Map literals
// =====================================================================

#[test]
fn map_literal_constructs_and_displays_in_order() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Show {"banana": 2, "apple": 1}.
"#,
        r#"{banana: 2, apple: 1}"#,
    );
}

#[test]
fn map_literal_reads_by_key() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Let m be {"a": 1, "b": 2}.
Show item "b" of m.
"#,
        "2",
    );
}

#[test]
fn map_literal_means_the_same_as_the_statement_form() {
    assert_same_meaning(
        r#"## Main
Let mutable m be a new Map of Text to Int.
Set item "a" of m to 1.
Set item "b" of m to 2.
Show item "a" of m.
Show item "b" of m.
"#,
        r#"## Main
Let m be {"a": 1, "b": 2}.
Show item "a" of m.
Show item "b" of m.
"#,
    );
}

#[test]
fn empty_map_literal_requires_key_and_value_types() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Let m be {} of Text to Int.
Show length of m.
"#,
        "0",
    );
}

// =====================================================================
// Set literals
// =====================================================================

#[test]
fn set_literal_constructs_with_distinct_elements() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Let s be {1, 2, 3, 2}.
Show length of s.
"#,
        "3",
    );
}

#[test]
fn set_literal_supports_membership() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Let s be {1, 2, 3}.
If s contains 2:
    Show "yes".
"#,
        "yes",
    );
}

#[test]
fn empty_set_literal_requires_element_type() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Let s be {} of Int.
Show length of s.
"#,
        "0",
    );
}

#[test]
fn set_literal_means_the_same_as_the_add_form() {
    assert_same_meaning(
        r#"## Main
Let mutable s be a new Set of Int.
Add 1 to s.
Add 2 to s.
If s contains 2:
    Show "yes".
"#,
        r#"## Main
Let s be {1, 2}.
If s contains 2:
    Show "yes".
"#,
    );
}

// =====================================================================
// Ambiguity threading: the intentional ambiguities keep resolving
// =====================================================================

#[test]
fn items_stays_usable_as_a_variable_beside_literals() {
    // `item`/`items` are keyword-vs-variable ambiguous by design — a set
    // literal bound to `items` must not disturb the lookahead.
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Let items be {1, 2, 3}.
Show length of items.
"#,
        "3",
    );
}

#[test]
fn literal_works_in_expression_position() {
    // The literal is a first-class expression, not a statement form — usable
    // directly as a call argument.
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Show length of {4, 5, 6, 5}.
"#,
        "3",
    );
}

// =====================================================================
// Interpolation braces are untouched
// =====================================================================

#[test]
fn string_interpolation_braces_unaffected() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Let x be 5.
Show "x is {x} and literal braces are {{ok}}".
"#,
        "x is 5 and literal braces are {ok}",
    );
}
