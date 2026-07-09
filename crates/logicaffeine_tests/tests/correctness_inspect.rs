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
