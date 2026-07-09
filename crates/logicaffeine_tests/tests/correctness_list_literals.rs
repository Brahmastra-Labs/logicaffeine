//! Part I correctness: `[1,2,3]` is a THREE-element list, not `[123]`.
//!
//! The thousands-separator lexer rule (a comma flanked by digits joins the
//! numeral) glues spaceless list literals into a single number — the audit's
//! `Let xs be [1,2,3].` → one element `[123]` row. The fix gates the glue on
//! bracket depth: inside `[` … `]` a digit-flanked comma is a separator,
//! UNLESS the current word is a money literal (`$125,000` keeps its commas
//! anywhere). Outside brackets nothing changes — prose numerals (`1,234`) and
//! money keep gluing.

mod common;
use common::{assert_compiled_equals_interpreted_eq, assert_interpreter_output};

// =====================================================================
// The Part I row: spaceless literals are element lists
// =====================================================================

#[test]
fn spaceless_list_literal_has_three_elements() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Let xs be [1,2,3].
Show length of xs.
"#,
        "3",
    );
}

#[test]
fn spaceless_list_literal_displays_all_elements() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Show [1,2,3].
"#,
        "[1, 2, 3]",
    );
}

#[test]
fn spaceless_multi_digit_elements_stay_separate() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Show [10,20,30].
"#,
        "[10, 20, 30]",
    );
}

#[test]
fn spaceless_literal_indexes_correctly() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Let xs be [1,2,3].
Show item 2 of xs.
"#,
        "2",
    );
}

// =====================================================================
// Locks: everything that works today keeps working
// =====================================================================

#[test]
fn spaced_list_literal_unchanged() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Show [1, 2, 3].
"#,
        "[1, 2, 3]",
    );
}

#[test]
fn money_literal_keeps_its_thousands_commas() {
    assert_interpreter_output(
        r#"## Main
Let price be $125,000.
If price equals $125,000:
    Show "glued".
"#,
        "glued",
    );
}

#[test]
fn bare_numeral_outside_brackets_still_glues() {
    assert_interpreter_output(
        r#"## Main
Let n be 1,234.
Show n.
"#,
        "1234",
    );
}
