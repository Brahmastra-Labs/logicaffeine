//! Part I correctness: a non-exhaustive `Inspect` is LOUD.
//!
//! The audit row: an `Inspect` whose arms don't cover the scrutinee's actual
//! variant (and has no `Otherwise`) silently did NOTHING — the unhandled case
//! fell through as a no-op, so a missing arm was invisible instead of a loud
//! failure. A total match or an `Otherwise` is required.

mod common;
use common::{assert_compiled_equals_interpreted_eq, run_interpreter, run_logos};

#[test]
fn unhandled_variant_without_otherwise_is_loud() {
    let src = r#"## A Color is one of:
    A Red.
    A Green.
    A Blue.

## Main
Let c be a new Blue.
Inspect c:
    When Red: Show "red".
    When Green: Show "green".
"#;
    let interp = run_interpreter(src);
    assert!(
        !interp.success,
        "interp must reject the unhandled Blue variant, got: {}",
        interp.output
    );
    let compiled = run_logos(src);
    assert!(
        !compiled.success,
        "compiled must reject the unhandled Blue variant, got: {}",
        compiled.stdout
    );
}

#[test]
fn otherwise_handles_the_rest() {
    assert_compiled_equals_interpreted_eq(
        r#"## A Color is one of:
    A Red.
    A Green.
    A Blue.

## Main
Let c be a new Blue.
Inspect c:
    When Red: Show "red".
    Otherwise: Show "other".
"#,
        "other",
    );
}

#[test]
fn a_matched_variant_still_runs() {
    assert_compiled_equals_interpreted_eq(
        r#"## A Color is one of:
    A Red.
    A Green.
    A Blue.

## Main
Let c be a new Green.
Inspect c:
    When Red: Show "red".
    When Green: Show "green".
    When Blue: Show "blue".
"#,
        "green",
    );
}

/// An `Inspect` that ends a function, on an item of a list the function holds: the compiled match reads the item
/// before the list goes out of scope.
#[test]
fn a_function_ending_in_an_inspect_of_an_item_of_its_own_list_compiles() {
    assert_compiled_equals_interpreted_eq(
        r#"## A Shape is one of:
    A Circle with r Int.
    A Dot.

## To firstRadius (n: Int) -> Int:
    Let shapes be [a new Circle with r n, a new Dot].
    Inspect item 1 of shapes:
        When Circle (r):
            Return r.
        Otherwise:
            Return 0.

## Main
Show firstRadius(3).
"#,
        "3",
    );
}
