mod common;
use common::{assert_exact_output, assert_interpreter_output, assert_c_output};

// =====================================================================
// sqrt — both interpreter and compiled
// =====================================================================

#[test]
fn interp_sqrt_float() {
    assert_interpreter_output(
        r#"## Main
Let x be 16.0.
Show sqrt(x).
"#,
        "4",
    );
}

#[test]
fn interp_sqrt_integer() {
    assert_interpreter_output(
        r#"## Main
Let x be 25.
Show sqrt(x).
"#,
        "5",
    );
}

#[test]
fn interp_sqrt_non_perfect() {
    assert_interpreter_output(
        r#"## Main
Let x be 2.0.
Show "{sqrt(x):.6}".
"#,
        "1.414214",
    );
}

#[test]
fn compiled_sqrt_float() {
    assert_exact_output(
        r#"## Main
Let x be 16.0.
Show sqrt(x).
"#,
        "4",
    );
}

#[test]
fn compiled_sqrt_integer() {
    assert_exact_output(
        r#"## Main
Let x be 25.
Show sqrt(x).
"#,
        "5",
    );
}

#[test]
fn compiled_sqrt_expression() {
    assert_exact_output(
        r#"## Main
Let a be 3.0.
Let b be 4.0.
Show sqrt(a * a + b * b).
"#,
        "5",
    );
}

// =====================================================================
// abs — both interpreter and compiled
// =====================================================================

#[test]
fn interp_abs_negative_int() {
    assert_interpreter_output(
        r#"## Main
Let x be 0 - 42.
Show abs(x).
"#,
        "42",
    );
}

#[test]
fn interp_abs_positive_int() {
    assert_interpreter_output(
        r#"## Main
Let x be 7.
Show abs(x).
"#,
        "7",
    );
}

#[test]
fn interp_abs_negative_float() {
    assert_interpreter_output(
        r#"## Main
Let x be 0.0 - 3.14.
Show abs(x).
"#,
        "3.14",
    );
}

#[test]
fn compiled_abs_negative_int() {
    assert_exact_output(
        r#"## Main
Let x be 0 - 42.
Show abs(x).
"#,
        "42",
    );
}

#[test]
fn compiled_abs_positive_int() {
    assert_exact_output(
        r#"## Main
Let x be 7.
Show abs(x).
"#,
        "7",
    );
}

#[test]
fn compiled_abs_negative_float() {
    assert_exact_output(
        r#"## Main
Let x be 0.0 - 3.14.
Show abs(x).
"#,
        "3.14",
    );
}

// =====================================================================
// min — both interpreter and compiled
// =====================================================================

#[test]
fn interp_min_integers() {
    assert_interpreter_output(
        r#"## Main
Show min(10, 3).
"#,
        "3",
    );
}

#[test]
fn interp_min_equal() {
    assert_interpreter_output(
        r#"## Main
Show min(5, 5).
"#,
        "5",
    );
}

#[test]
fn compiled_min_integers() {
    assert_exact_output(
        r#"## Main
Show min(10, 3).
"#,
        "3",
    );
}

#[test]
fn compiled_min_negative() {
    assert_exact_output(
        r#"## Main
Let a be 0 - 5.
Let b be 3.
Show min(a, b).
"#,
        "-5",
    );
}

// =====================================================================
// max — both interpreter and compiled
// =====================================================================

#[test]
fn interp_max_integers() {
    assert_interpreter_output(
        r#"## Main
Show max(10, 3).
"#,
        "10",
    );
}

#[test]
fn interp_max_equal() {
    assert_interpreter_output(
        r#"## Main
Show max(5, 5).
"#,
        "5",
    );
}

#[test]
fn compiled_max_integers() {
    assert_exact_output(
        r#"## Main
Show max(10, 3).
"#,
        "10",
    );
}

#[test]
fn compiled_max_negative() {
    assert_exact_output(
        r#"## Main
Let a be 0 - 5.
Let b be 3.
Show max(a, b).
"#,
        "3",
    );
}

// =====================================================================
// floor — both interpreter and compiled
// =====================================================================

#[test]
fn interp_floor_positive() {
    assert_interpreter_output(
        r#"## Main
Let x be 3.7.
Show floor(x).
"#,
        "3",
    );
}

#[test]
fn interp_floor_negative() {
    assert_interpreter_output(
        r#"## Main
Let x be 0.0 - 3.2.
Show floor(x).
"#,
        "-4",
    );
}

#[test]
fn interp_floor_integer_passthrough() {
    assert_interpreter_output(
        r#"## Main
Let x be 5.
Show floor(x).
"#,
        "5",
    );
}

#[test]
fn compiled_floor_positive() {
    assert_exact_output(
        r#"## Main
Let x be 3.7.
Show floor(x).
"#,
        "3",
    );
}

#[test]
fn compiled_floor_negative() {
    assert_exact_output(
        r#"## Main
Let x be 0.0 - 3.2.
Show floor(x).
"#,
        "-4",
    );
}

// =====================================================================
// ceil — both interpreter and compiled
// =====================================================================

#[test]
fn interp_ceil_positive() {
    assert_interpreter_output(
        r#"## Main
Let x be 3.2.
Show ceil(x).
"#,
        "4",
    );
}

#[test]
fn interp_ceil_negative() {
    assert_interpreter_output(
        r#"## Main
Let x be 0.0 - 3.7.
Show ceil(x).
"#,
        "-3",
    );
}

#[test]
fn interp_ceil_integer_passthrough() {
    assert_interpreter_output(
        r#"## Main
Let x be 5.
Show ceil(x).
"#,
        "5",
    );
}

#[test]
fn compiled_ceil_positive() {
    assert_exact_output(
        r#"## Main
Let x be 3.2.
Show ceil(x).
"#,
        "4",
    );
}

#[test]
fn compiled_ceil_negative() {
    assert_exact_output(
        r#"## Main
Let x be 0.0 - 3.7.
Show ceil(x).
"#,
        "-3",
    );
}

// =====================================================================
// round — both interpreter and compiled
// =====================================================================

#[test]
fn interp_round_up() {
    assert_interpreter_output(
        r#"## Main
Let x be 3.7.
Show round(x).
"#,
        "4",
    );
}

#[test]
fn interp_round_down() {
    assert_interpreter_output(
        r#"## Main
Let x be 3.2.
Show round(x).
"#,
        "3",
    );
}

#[test]
fn interp_round_integer_passthrough() {
    assert_interpreter_output(
        r#"## Main
Let x be 5.
Show round(x).
"#,
        "5",
    );
}

#[test]
fn compiled_round_up() {
    assert_exact_output(
        r#"## Main
Let x be 3.7.
Show round(x).
"#,
        "4",
    );
}

#[test]
fn compiled_round_down() {
    assert_exact_output(
        r#"## Main
Let x be 3.2.
Show round(x).
"#,
        "3",
    );
}

// =====================================================================
// pow — both interpreter and compiled
// =====================================================================

#[test]
fn interp_pow_integers() {
    assert_interpreter_output(
        r#"## Main
Show pow(2, 10).
"#,
        "1024",
    );
}

#[test]
fn interp_pow_float_base() {
    assert_interpreter_output(
        r#"## Main
Let x be 2.0.
Show pow(x, 3).
"#,
        "8",
    );
}

#[test]
fn interp_pow_zero() {
    assert_interpreter_output(
        r#"## Main
Show pow(5, 0).
"#,
        "1",
    );
}

#[test]
fn compiled_pow_integers() {
    assert_exact_output(
        r#"## Main
Show pow(2, 10).
"#,
        "1024",
    );
}

#[test]
fn compiled_pow_float_base() {
    assert_exact_output(
        r#"## Main
Let x be 2.0.
Show pow(x, 3).
"#,
        "8",
    );
}

#[test]
fn compiled_pow_zero() {
    assert_exact_output(
        r#"## Main
Show pow(5, 0).
"#,
        "1",
    );
}

// =====================================================================
// Combined / realistic usage
// =====================================================================

#[test]
fn interp_pythagorean_distance() {
    assert_interpreter_output(
        r#"## Main
Let dx be 3.0.
Let dy be 4.0.
Let dist be sqrt(dx * dx + dy * dy).
Show dist.
"#,
        "5",
    );
}

#[test]
fn compiled_pythagorean_distance() {
    assert_exact_output(
        r#"## Main
Let dx be 3.0.
Let dy be 4.0.
Let dist be sqrt(dx * dx + dy * dy).
Show dist.
"#,
        "5",
    );
}

#[test]
fn interp_clamp_with_min_max() {
    assert_interpreter_output(
        r#"## Main
Let value be 150.
Let lo be 0.
Let hi be 100.
Let clamped be min(max(value, lo), hi).
Show clamped.
"#,
        "100",
    );
}

#[test]
fn compiled_clamp_with_min_max() {
    assert_exact_output(
        r#"## Main
Let value be 150.
Let lo be 0.
Let hi be 100.
Let clamped be min(max(value, lo), hi).
Show clamped.
"#,
        "100",
    );
}

#[test]
fn interp_floor_ceil_round_series() {
    assert_interpreter_output(
        r#"## Main
Let x be 2.5.
Show floor(x).
Show ceil(x).
Show round(x).
"#,
        "2\n3\n3",
    );
}

#[test]
fn compiled_floor_ceil_round_series() {
    assert_exact_output(
        r#"## Main
Let x be 2.5.
Show floor(x).
Show ceil(x).
Show round(x).
"#,
        "2\n3\n3",
    );
}

// =====================================================================
// C codegen tests
// =====================================================================

#[test]
fn c_sqrt() {
    assert_c_output(
        r#"## Main
Let x be 16.0.
Show sqrt(x).
"#,
        "4",
    );
}

#[test]
fn c_abs() {
    assert_c_output(
        r#"## Main
Let x be 0.0 - 42.0.
Show abs(x).
"#,
        "42",
    );
}

#[test]
fn c_min_max() {
    assert_c_output(
        r#"## Main
Show min(10, 3).
Show max(10, 3).
"#,
        "3\n10",
    );
}

#[test]
fn c_floor_ceil_round() {
    assert_c_output(
        r#"## Main
Let x be 3.7.
Show floor(x).
Show ceil(x).
Show round(x).
"#,
        "3\n4\n4",
    );
}

#[test]
fn c_pow() {
    assert_c_output(
        r#"## Main
Show pow(2, 10).
"#,
        "1024",
    );
}
