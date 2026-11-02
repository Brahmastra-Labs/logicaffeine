//! The stdlib `Result` (prelude `core.lg`: `A Result of [T] and [E] is either:
//! An Ok (value: T). An Err (error: E).`) must be constructible and matchable
//! from user code on the playground interpreter — exactly like a self-contained
//! user-defined tagged union already is. Constructing `a new Ok with value …`
//! used to panic the interpreter (the prelude-generic path), which forced the
//! Syntax Guide to teach a workaround enum instead of the real thing.

use logicaffeine_compile::interpret_for_ui_sync;

const PROGRAM: &str = r#"## To checked_div (a: Int) and (b: Int) -> Result of Int and Text:
    If b equals 0:
        Return a new Err with error "divide by zero".
    Return a new Ok with value (a / b).

## Main
Let r be checked_div(10, 2).
Inspect r:
    When Ok (v):
        Show "got " + v.
    When Err (e):
        Show "error: " + e.
Let bad be checked_div(1, 0).
Inspect bad:
    When Ok (v):
        Show "got " + v.
    When Err (e):
        Show "error: " + e."#;

#[test]
fn stdlib_result_constructs_and_matches_on_the_interpreter() {
    let r = interpret_for_ui_sync(PROGRAM);
    assert_eq!(
        r.error, None,
        "stdlib Result must interpret cleanly, got error: {:?}",
        r.error
    );
    assert_eq!(r.lines, vec!["got 5", "error: divide by zero"]);
}

/// `Option` is BUILTIN-backed: the `some x` / `none` literals are its
/// constructors (codegen lowers them to `std::option::Option`, so the prelude
/// deliberately declares no competing enum). A `-> Option of Int` function
/// round-trips through the literals and `Inspect`.
const OPTION_PROGRAM: &str = r#"## To find_even (xs: Seq of Int) -> Option of Int:
    Repeat for x in xs:
        If x % 2 equals 0:
            Return some x.
    Return none.

## Main
Let hit be find_even([3, 4, 5]).
Inspect hit:
    When Some (v):
        Show "found " + v.
    When None:
        Show "none".
Let miss be find_even([3, 5, 7]).
Inspect miss:
    When Some (v):
        Show "found " + v.
    When None:
        Show "none"."#;

/// The `some x` / `none` LITERALS and the prelude constructors are the same
/// values — a literal-built Option must match `When Some (v):` / `When None:`
/// arms identically to `a new Some with value x`. One Option, one shape.
const LITERAL_PROGRAM: &str = r#"## Main
Let hit be some 4.
Inspect hit:
    When Some (v):
        Show "found " + v.
    When None:
        Show "none".
Let miss be none.
Inspect miss:
    When Some (v):
        Show "found " + v.
    When None:
        Show "none"."#;

#[test]
fn option_literals_match_when_arms() {
    let r = interpret_for_ui_sync(LITERAL_PROGRAM);
    assert_eq!(
        r.error, None,
        "some/none literals must Inspect like the prelude Option, got: {:?}",
        r.error
    );
    assert_eq!(r.lines, vec!["found 4", "none"]);
}

#[test]
fn stdlib_option_constructs_and_matches_on_the_interpreter() {
    let r = interpret_for_ui_sync(OPTION_PROGRAM);
    assert_eq!(
        r.error, None,
        "stdlib Option must interpret cleanly, got error: {:?}",
        r.error
    );
    assert_eq!(r.lines, vec!["found 4", "none"]);
}
