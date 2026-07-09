//! Part I correctness: ONE float-display path across every engine.
//!
//! The rows under test (LANGUAGE_SMELLS.md Part I):
//!   - `Show 1.0 / 3.0.` printed `0.333333` in interp/VM but full precision compiled.
//!   - `Show 0.0000001.` rendered a NONZERO value as `0` in the interpreter.
//!   - A typed literal like `3.141592653589793` was silently truncated to 6 digits.
//!
//! The spec: shortest round-trip decimal (Rust `{}` Display for f64), identical on
//! the tree-walker, the bytecode VM (shadow-oracled by `run_interpreter`), and the
//! compiled binary. Never scientific notation, never a nonzero value as `0`,
//! integral floats stay bare (`2.0` shows as `2`).

mod common;
use common::{assert_compiled_equals_interpreted_eq, assert_interpreter_output};

// =====================================================================
// The Part I rows themselves
// =====================================================================

#[test]
fn float_division_shows_shortest_roundtrip_everywhere() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Show 1.0 / 3.0.
"#,
        "0.3333333333333333",
    );
}

#[test]
fn tiny_nonzero_float_never_renders_as_zero() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Show 0.0000001.
"#,
        "0.0000001",
    );
}

#[test]
fn typed_pi_echoes_the_stored_value() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Show 3.141592653589793.
"#,
        "3.141592653589793",
    );
}

#[test]
fn float_sum_artifact_is_visible_not_hidden() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Show 0.1 + 0.2.
"#,
        "0.30000000000000004",
    );
}

// =====================================================================
// Churn-minimizing locks: short/integral floats keep today's rendering
// =====================================================================

#[test]
fn integral_float_stays_bare() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Show 2.0.
"#,
        "2",
    );
}

#[test]
fn short_float_unchanged() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Show 1.5.
"#,
        "1.5",
    );
}

#[test]
fn negative_float_displays_with_sign() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Show 0.0 - 1.5.
"#,
        "-1.5",
    );
}

// =====================================================================
// Floats reached through containers and interpolation take the same path
// =====================================================================

#[test]
fn floats_inside_a_seq_use_the_shared_path() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Let mutable xs be a new Seq of Float.
Push 1.5 to xs.
Push 1.0 / 3.0 to xs.
Show xs.
"#,
        "[1.5, 0.3333333333333333]",
    );
}

#[test]
fn floats_inside_interpolation_use_the_shared_path() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Let x be 1.0 / 3.0.
Show "x is {x}".
"#,
        "x is 0.3333333333333333",
    );
}

// =====================================================================
// Explicit format specs are USER-REQUESTED formatting — they keep their
// exact meaning. (`{v:.6}` stays 6 digits; only the bare default changed.)
// =====================================================================

#[test]
fn explicit_precision_spec_is_untouched() {
    assert_interpreter_output(
        r#"## Main
Let x be 1.0 / 3.0.
Show "{x:.6}".
"#,
        "0.333333",
    );
}
