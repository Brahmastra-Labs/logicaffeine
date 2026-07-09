//! Wave 7 keystones: inline guard `If c: <stmt>.` — a single-statement body
//! on the same line as the condition (no indented block). Parser-only.

mod common;
use common::assert_compiled_equals_interpreted_eq;

#[test]
fn inline_if_returns_early() {
    assert_compiled_equals_interpreted_eq(
        r#"## To classify (n: Int) -> Int:
    If n is even: Return 0.
    Return 1.

## Main
Show classify(4).
Show classify(7).
"#,
        "0\n1",
    );
}

#[test]
fn inline_if_show() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Let x be 5.
If x is odd: Show "odd".
Show "done".
"#,
        "odd\ndone",
    );
}

#[test]
fn inline_if_not_taken() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Let x be 4.
If x is odd: Show "odd".
Show "done".
"#,
        "done",
    );
}

#[test]
fn inline_if_with_otherwise() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Let x be 4.
If x is odd: Show "odd".
Otherwise: Show "even".
"#,
        "even",
    );
}

#[test]
fn block_if_still_works() {
    // The indented block form must remain intact.
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Let x be 5.
If x is odd:
    Show "odd".
    Show "yes".
"#,
        "odd\nyes",
    );
}
