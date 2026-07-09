//! Wave 7 keystones: chained comparisons `lo <= x <= hi` desugar to
//! `(lo <= x) and (x <= hi)` — the symbolic forms (`< <= > >=`), the way
//! math and Python read. Parser-only, no new AST node.

mod common;
use common::assert_compiled_equals_interpreted_eq;

#[test]
fn chained_le_inside() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Let i be 5.
If 0 <= i <= 10:
    Show "in".
Otherwise:
    Show "out".
"#,
        "in",
    );
}

#[test]
fn chained_le_outside() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Let i be 15.
If 0 <= i <= 10:
    Show "in".
Otherwise:
    Show "out".
"#,
        "out",
    );
}

#[test]
fn chained_strict_lt() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Let x be 5.
If 0 < x < 5:
    Show "in".
Otherwise:
    Show "out".
"#,
        "out",
    );
}

#[test]
fn chained_descending() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Let x be 5.
If 10 >= x >= 1:
    Show "in".
Otherwise:
    Show "out".
"#,
        "in",
    );
}

#[test]
fn plain_single_comparison_unaffected() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Let x be 5.
If x <= 10:
    Show "ok".
Otherwise:
    Show "no".
"#,
        "ok",
    );
}
