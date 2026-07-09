//! Wave 7: trailing-condition return `Return X if c.` → `If c: Return X.`,
//! and `Repeat forever:` (infinite loop with Break to exit).

mod common;
use common::assert_compiled_equals_interpreted_eq;

#[test]
fn trailing_return_if_taken() {
    assert_compiled_equals_interpreted_eq(
        r#"## To f (n: Int) -> Int:
    Return 0 if n is even.
    Return 1.

## Main
Show f(4).
Show f(7).
"#,
        "0\n1",
    );
}

#[test]
fn trailing_return_bare() {
    // `Return if c.` (no value) also guards.
    assert_compiled_equals_interpreted_eq(
        r#"## To g (n: Int) -> Int:
    Let mutable r be 10.
    Return r if n is odd.
    Set r to 20.
    Return r.

## Main
Show g(3).
Show g(4).
"#,
        "10\n20",
    );
}

#[test]
fn repeat_forever_with_break() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Let mutable i be 0.
Repeat forever:
    i += 1.
    If i == 5:
        Break.
Show i.
"#,
        "5",
    );
}
